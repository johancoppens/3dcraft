//! The renderer on a device someone else owns: pipelines, targets and the
//! per-frame numbers.

use crate::GpuError;
use crate::cut;
use crate::mesh::{GpuMesh, MeshPart};
use crate::peel;
use crate::readback;
use crate::studio::{self, Passes, Studio, StudioGpu, StudioTargets};
use craft3d_math::{Bbox3, Transform};
use craft3d_render::{
    Camera, Cut, FaceColors, Frame, MAX_SAMPLES, MAX_SIDE, MAX_SSAA, NO_FACE, Projection,
    RenderError, RenderParams, SHADE_AMBIENT, SHADE_DIFFUSE, SHADE_FILL, SHADE_HIGHLIGHT_AMBIENT,
    SHADE_HIGHLIGHT_DIFFUSE, SHADE_HIGHLIGHT_FILL, SHADE_SHININESS, SHADE_SPECULAR, ViewAxes,
};
use craft3d_tess::Mesh;
use std::sync::{Arc, Mutex, PoisonError};

/// The colour targets store the bytes of [`Frame::rgb`] as they are: the
/// shading works on display values, as the CPU renderer's does.
pub(crate) const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Red: the face id. Green: the bits of the view distance as an `f32`. An
/// integer target is renderable on every backend (a float one is not on
/// OpenGL ES) and stores both without conversion.
pub(crate) const DATA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The depth of [`DEPTH_FORMAT`] with a stencil, for the scene drawn in two
/// steps ([`Step`]).
pub(crate) const DEPTH_STENCIL_FORMAT: wgpu::TextureFormat =
    wgpu::TextureFormat::Depth32FloatStencil8;

/// Number of `f32` in the scene uniform block: eight `vec4` for the view
/// and the shading, one for the highlight's colour, one for its light,
/// four for its runs of faces and four for the section.
pub(crate) const SCENE_FLOATS: usize = 140;

/// Face ids are compared as `f32` in the shader: exact below this.
const MAX_HIGHLIGHT_FACE: u32 = 1 << 24;

/// The vertex layout of [`GpuMesh`]: position, normal, the triangle's own
/// normal, the face id, its colour, its finish, its layers, and the
/// curvature map's value with whether there is one.
pub(crate) const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 8] = [
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
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 40,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 44,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 48,
        shader_location: 6,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 52,
        shader_location: 7,
    },
];

/// Runs `work` with wgpu's validation, out-of-memory and internal errors
/// captured instead of raised: without a scope wgpu's default is to panic.
pub(crate) fn guarded<T>(device: &wgpu::Device, work: impl FnOnce() -> T) -> Result<T, GpuError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let out = work();
        // On native wgpu these futures are ready at once. Scopes are popped
        // innermost first.
        let validation = pollster::block_on(validation.pop());
        let memory = pollster::block_on(memory.pop());
        let internal = pollster::block_on(internal.pop());
        match validation.or(memory).or(internal) {
            Some(error) => Err(GpuError::Device(error.to_string())),
            None => Ok(out),
        }
    }
    // WebGPU reports these to the console without unwinding, and its scopes
    // can only be awaited.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = device;
        Ok(work())
    }
}

/// The CPU renderer's checks on the image size, then the device's.
pub(crate) fn check_params(params: &RenderParams, max_texture_side: u32) -> Result<(), GpuError> {
    let (width, height, ssaa) = (params.width, params.height, params.ssaa);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(RenderError::InvalidSize { width, height }.into());
    }
    if ssaa == 0 || ssaa > MAX_SSAA {
        return Err(RenderError::InvalidSsaa(ssaa).into());
    }
    let samples = u64::from(width) * u64::from(height) * u64::from(ssaa) * u64::from(ssaa);
    if samples > MAX_SAMPLES {
        return Err(RenderError::TooManySamples { samples }.into());
    }
    let side = u64::from(width.max(height)) * u64::from(ssaa);
    if side > u64::from(max_texture_side) {
        return Err(GpuError::TargetTooLarge {
            side,
            limit: max_texture_side,
        });
    }
    Ok(())
}

/// Near and far distance along the view direction that enclose `bounds`
/// with a margin: the tighter the range, the finer the depth buffer. The
/// CPU renderer's own function (`craft3d_render::depth_range`), so that
/// both clip at the same near plane.
fn depth_range(axes: &ViewAxes, bounds: &Bbox3, perspective: bool) -> (f64, f64) {
    craft3d_render::depth_range(axes.eye, axes.forward, bounds, perspective)
}

/// The numbers of `scene.wgsl`'s uniform block, computed in `f64` and
/// narrowed last.
pub(crate) fn scene_uniforms(
    mesh: &GpuMesh,
    camera: &Camera,
    params: &RenderParams,
    cut: Option<&Cut>,
) -> Result<[f32; SCENE_FLOATS], GpuError> {
    let axes = camera.axes()?;
    let perspective = matches!(axes.projection, Projection::Perspective { .. });
    // The same ratio as the CPU renderer's: the supersampled sizes cancel.
    let aspect = f64::from(params.width) / f64::from(params.height);
    let eye = axes.eye - mesh.origin;
    // The cap of a section may lie in the margin around the mesh.
    let (near, far) = match cut {
        Some(cut) if cut.cap => depth_range(&axes, &cut::cap_bounds(mesh), perspective),
        _ => depth_range(&axes, &mesh.bounds, perspective),
    };
    // Reversed depth (`scene.wgsl`, `vs_main`): near / dist in
    // perspective, (far - dist) / (far - near) in orthographic.
    let depth = if perspective {
        [near, 1.0]
    } else {
        [far, 1.0 / (far - near)]
    };
    let light = axes.light();
    let l = light.unwrap_or(craft3d_math::Vec3::ZERO);
    let fill = axes.fill_light();
    let f = fill.unwrap_or(craft3d_math::Vec3::ZERO);
    let [r, g, b] = params.color.map(f64::from);
    let [hr, hg, hb] = params.highlight.color.map(f64::from);
    // Two runs per vector; a run that is not there is first 1, last 0.
    let mut runs = [[1.0, 0.0, 1.0, 0.0]; 4];
    for (k, [first, last]) in params.highlight.ranges().enumerate() {
        if last >= MAX_HIGHLIGHT_FACE {
            return Err(GpuError::OutOfRange("the highlighted faces"));
        }
        if let Some(run) = runs.get_mut(k / 2).and_then(|v| v.get_mut(k % 2 * 2..)) {
            for (slot, id) in run.iter_mut().zip([first, last]) {
                *slot = f64::from(id);
            }
        }
    }
    let [run0, run1, run2, run3] = runs;
    let [section, section_cap, section_lo, section_hi] = cut::section_uniforms(mesh, cut);
    let wide = [
        [
            axes.right.x,
            axes.right.y,
            axes.right.z,
            1.0 / (axes.half * aspect),
        ],
        [axes.up.x, axes.up.y, axes.up.z, 1.0 / axes.half],
        [
            axes.forward.x,
            axes.forward.y,
            axes.forward.z,
            if perspective { 1.0 } else { 0.0 },
        ],
        [eye.x, eye.y, eye.z, 0.0],
        [l.x, l.y, l.z, if light.is_some() { 1.0 } else { 0.0 }],
        [depth[0], depth[1], 0.0, 0.0],
        [r, g, b, 0.0],
        [
            SHADE_AMBIENT,
            SHADE_DIFFUSE,
            255.0 * SHADE_SPECULAR,
            f64::from(SHADE_SHININESS.max(0)),
        ],
        [hr, hg, hb, 0.0],
        [
            SHADE_HIGHLIGHT_AMBIENT,
            SHADE_HIGHLIGHT_DIFFUSE,
            SHADE_HIGHLIGHT_FILL,
            0.0,
        ],
        run0,
        run1,
        run2,
        run3,
        section,
        section_cap,
        section_lo,
        section_hi,
        // The fill light (scene.wgsl's `fill`): its direction and weight;
        // the weight is 0 when there is none.
        [f.x, f.y, f.z, if fill.is_some() { SHADE_FILL } else { 0.0 }],
    ];
    let analysis = analysis_uniforms(params, &axes, &mesh.bounds);
    let wide = wide.iter().chain(analysis.iter());
    let mut out = [0.0_f32; SCENE_FLOATS];
    for (slot, value) in out.iter_mut().zip(wide.flatten()) {
        // Deliberate narrowing at the GPU boundary.
        *slot = *value as f32;
        if !slot.is_finite() {
            return Err(GpuError::OutOfRange("the camera"));
        }
    }
    // The range must survive the narrowing, or every depth would be NaN.
    if (far as f32) <= (near as f32) {
        return Err(GpuError::OutOfRange("the depth range of the view"));
    }
    Ok(out)
}

/// The analysis part of `scene.wgsl`'s uniform block (its `analysis…`
/// fields): [`craft3d_render::analysis::Resolved`] for the view; all
/// zero (mode 0) without an analysis.
fn analysis_uniforms(
    params: &RenderParams,
    axes: &craft3d_render::ViewAxes,
    bounds: &craft3d_math::Bbox3,
) -> [[f64; 4]; 16] {
    let mut out = [[0.0; 4]; 16];
    let Some(analysis) = params.analysis else {
        return out;
    };
    let r = craft3d_render::analysis::Resolved::new(&analysis, axes, bounds);
    let c = r.colors;
    let rgb = |v: [u8; 3]| {
        let [r, g, b] = v.map(f64::from);
        [r, g, b, 0.0]
    };
    let [r0, r1, r2, r3, r4] = c.ramp;
    out = [
        [f64::from(r.mode.code()), r.count, r.width, r.sin_min],
        [r.axis.x, r.axis.y, r.axis.z, r.spacing],
        [r.first.x, r.first.y, r.first.z, r.lo],
        [r.second.x, r.second.y, r.second.z, r.hi],
        [r.point.x, r.point.y, r.point.z, r.offset],
        rgb(c.dark),
        rgb(c.light),
        rgb(c.draft_in),
        rgb(c.draft_out),
        rgb(c.draft_shallow),
        // The neutral colour, with the grazing bound of highlight lines.
        {
            let [r0, g0, b0] = c.none.map(f64::from);
            [r0, g0, b0, r.grazing]
        },
        rgb(r0),
        rgb(r1),
        rgb(r2),
        rgb(r3),
        rgb(r4),
    ];
    out
}

pub(crate) fn float_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The textures of one image size.
pub(crate) struct Targets {
    pub width: u32,
    pub height: u32,
    ssaa: u32,
    samples_color: wgpu::TextureView,
    samples_data: wgpu::TextureView,
    /// The front's depth (and stencil, in two steps), as the attachment.
    samples_depth: wgpu::TextureView,
    /// The same depth alone, for the passes that read it.
    samples_depth_read: wgpu::TextureView,
    pub color: wgpu::Texture,
    pub data: wgpu::Texture,
    color_view: wgpu::TextureView,
    data_view: wgpu::TextureView,
    resolve_bind: wgpu::BindGroup,
    present_bind: wgpu::BindGroup,
    /// What a see-through picture needs besides; made for the first one.
    peel: Option<PeelTargets>,
    /// What a cut-open picture needs besides; made for the first one.
    cut: Option<CutTargets>,
}

/// The count of a cut-open picture (`cut.rs`), at the size of the samples.
struct CutTargets {
    parity: wgpu::TextureView,
    cap_bind: wgpu::BindGroup,
    /// The count with the front's depth buffer, for the cap behind the
    /// front of a see-through picture; made for the first one.
    cap_behind_bind: Option<wgpu::BindGroup>,
}

/// The pipelines of a cut-open picture (`cut.rs`), made for the first one.
struct CutPipelines {
    /// Counts what is left out.
    parity: wgpu::RenderPipeline,
    /// The scene without what is cut away.
    scene: Drawing,
    /// The cap, also as the front of a see-through picture.
    cap: Drawing,
    cap_layout: wgpu::BindGroupLayout,
    /// For a see-through picture; `None` on a device that draws none.
    peeled: Option<CutPeeled>,
}

struct CutPeeled {
    front: Drawing,
    behind: Drawing,
    cap_behind: Drawing,
    cap_behind_layout: wgpu::BindGroupLayout,
}

impl CutPipelines {
    fn build(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        peel_layout: Option<&wgpu::BindGroupLayout>,
        two: bool,
    ) -> CutPipelines {
        let module = cut::module(device, false, false);
        let cap_layout = cut::cap_layout(device, false, false);
        let frame = [target(COLOR_FORMAT), target(DATA_FORMAT)];
        let parity = cut::pipeline(
            device,
            &module,
            "fs_parity",
            &[Some(scene_layout)],
            &[cut::parity_target()],
            1,
            false,
        );
        let drawing = |entry: &str, groups: &[Option<&wgpu::BindGroupLayout>], targets: &[_]| {
            Drawing::build(two, |step| {
                cut::pipeline_in(device, &module, entry, groups, targets, 1, Some(step))
            })
        };
        let scene = drawing("fs_main_cut", &[Some(scene_layout)], &frame);
        let cap = drawing("fs_cap", &[Some(scene_layout), Some(&cap_layout)], &frame);
        let peeled = peel_layout.map(|peel_layout| {
            let cap_behind_layout = cut::cap_layout(device, false, true);
            CutPeeled {
                front: drawing("fs_front_cut", &[Some(scene_layout)], &frame),
                behind: drawing(
                    "fs_behind_cut",
                    &[Some(scene_layout), Some(peel_layout)],
                    &[target(COLOR_FORMAT)],
                ),
                cap_behind: drawing(
                    "fs_cap_behind",
                    &[Some(scene_layout), Some(&cap_behind_layout)],
                    &[target(COLOR_FORMAT)],
                ),
                cap_behind_layout,
            }
        });
        CutPipelines {
            parity,
            scene,
            cap,
            cap_layout,
            peeled,
        }
    }
}

/// The targets of the layer behind the front one, at the size of the
/// samples.
struct PeelTargets {
    behind: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// The front's depth buffer, for the pass behind it.
    peel_bind: wgpu::BindGroup,
    blend_bind: wgpu::BindGroup,
}

/// The pipelines of a see-through picture (`peel.rs`).
struct SeeThrough {
    /// The scene with the alpha that says what is seen through.
    front: Drawing,
    behind: Drawing,
    blend_pipeline: wgpu::RenderPipeline,
    peel_layout: wgpu::BindGroupLayout,
    blend_layout: wgpu::BindGroupLayout,
    table: peel::Table,
    /// The nearest sampler the pass behind reads the front's depth
    /// through on OpenGL (`peel::sampled_front`).
    front_sampler: wgpu::Sampler,
}

/// A renderer on a device and queue the caller owns. It keeps its
/// pipelines and one set of targets, which it reallocates when the image
/// size or supersampling changes.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Whether the passes over the triangles are drawn in two steps
    /// ([`Step`]).
    two: bool,
    scene: Drawing,
    scene_uniforms: wgpu::Buffer,
    scene_layout: wgpu::BindGroupLayout,
    scene_bind: wgpu::BindGroup,
    resolve_pipeline: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    resolve_uniforms: wgpu::Buffer,
    present_shader: wgpu::ShaderModule,
    present_layout: wgpu::BindGroupLayout,
    present_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    targets: Option<Targets>,
    /// What draws a see-through picture, or why this device does not.
    see_through: Result<SeeThrough, String>,
    /// What draws a cut-open picture, or why this device does not; made
    /// for the first one.
    cut: Option<Result<CutPipelines, String>>,
    /// The triangle of the cap, written every cut-open frame.
    cap_buffer: wgpu::Buffer,
    /// What draws the studio look, or why this device does not; made for
    /// the first studio picture.
    studio: Option<Result<StudioReadback, String>>,
    /// The buffers frames are read back through, kept between frames.
    staging: Mutex<Option<readback::Staging>>,
    drawn: bool,
    /// Identifies this renderer in the meshes it uploads.
    token: Arc<()>,
}

/// The device feature for drawing in two steps ([`Step`]).
pub(crate) const TWO_STEPS_FEATURE: wgpu::Features = wgpu::Features::DEPTH32FLOAT_STENCIL8;

/// The step a pass over the triangles is drawn in. In one step a sample is
/// written by every fragment nearer than those before it. In two steps
/// every sample is written once, by the first fragment drawn of those
/// nearest to the eye, which is the same picture: first the depth alone
/// (the colour targets masked), then the colour where the depth equals the
/// nearest and the stencil is still 0, which the write then raises. On
/// the AMD Radeon 8060S (Mesa 26.2, RADV and radeonsi alike) one of the
/// several writes of a sample in one step lands out of order now and then
/// (one frame of the teapot in about 250, more seen through) while its
/// delta colour compression is on, and the surface behind stays
/// (docs/design/rendering.md, *A frame that differed once*); with one
/// write per sample there is no order to lose. Both steps run the same
/// shaders, so a fragment's depth is the same number in both (the
/// position is `@invariant`; on OpenGL the fragment shader writes the
/// depth with the same arithmetic) and a fragment discarded in one is
/// discarded in the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    One,
    Depth,
    Color,
}

impl Step {
    /// The depth (and stencil) state of a pipeline of this step:
    /// `Greater`, because the depth is reversed (`scene.wgsl`), so that on
    /// a tie the triangle drawn first stays.
    pub(crate) fn depth_stencil(self) -> wgpu::DepthStencilState {
        // Where the depth is the nearest and nothing was written yet
        // (reference 0), and then not again.
        let once = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::IncrementClamp,
        };
        let (format, depth_write, depth_compare, stencil) = match self {
            Step::One => (
                DEPTH_FORMAT,
                true,
                wgpu::CompareFunction::Greater,
                wgpu::StencilState::default(),
            ),
            Step::Depth => (
                DEPTH_STENCIL_FORMAT,
                true,
                wgpu::CompareFunction::Greater,
                wgpu::StencilState::default(),
            ),
            Step::Color => (
                DEPTH_STENCIL_FORMAT,
                false,
                wgpu::CompareFunction::Equal,
                wgpu::StencilState {
                    front: once,
                    back: once,
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
            ),
        };
        wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(depth_compare),
            stencil,
            bias: wgpu::DepthBiasState::default(),
        }
    }

    /// `targets` as this step writes them: not at all in the depth step.
    pub(crate) fn targets(
        self,
        targets: &[Option<wgpu::ColorTargetState>],
    ) -> Vec<Option<wgpu::ColorTargetState>> {
        targets
            .iter()
            .map(|t| {
                t.clone().map(|mut t| {
                    if self == Step::Depth {
                        t.write_mask = wgpu::ColorWrites::empty();
                    }
                    t
                })
            })
            .collect()
    }
}

/// A pass over the triangles: one pipeline, or one per [`Step`].
pub(crate) enum Drawing {
    One(wgpu::RenderPipeline),
    Two {
        depth: wgpu::RenderPipeline,
        color: wgpu::RenderPipeline,
    },
}

impl Drawing {
    /// The pipelines `make` makes for each step, in two steps when `two`.
    pub(crate) fn build(two: bool, make: impl Fn(Step) -> wgpu::RenderPipeline) -> Drawing {
        if two {
            Drawing::Two {
                depth: make(Step::Depth),
                color: make(Step::Color),
            }
        } else {
            Drawing::One(make(Step::One))
        }
    }
}

/// One draw of a pass: its drawing and what sets the rest (bind groups,
/// vertex buffers) and draws, given the pipeline of the step.
type Draw<'a> = (
    &'a Drawing,
    &'a dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline),
);

/// Draws `draws` in order; those in two steps all for their depth first,
/// then all for their colour.
fn draw_in_steps(pass: &mut wgpu::RenderPass<'_>, draws: &[Draw<'_>]) {
    for (drawing, draw) in draws {
        match drawing {
            Drawing::One(pipeline)
            | Drawing::Two {
                depth: pipeline, ..
            } => draw(pass, pipeline),
        }
    }
    pass.set_stencil_reference(0);
    for (drawing, draw) in draws {
        if let Drawing::Two { color, .. } = drawing {
            draw(pass, color);
        }
    }
}

/// The depth attachment of a pass over the triangles: cleared, kept when
/// `store`, with its stencil cleared too when it has one (`two`).
fn depth_attachment(
    view: &wgpu::TextureView,
    two: bool,
    store: bool,
) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    let store = if store {
        wgpu::StoreOp::Store
    } else {
        wgpu::StoreOp::Discard
    };
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(0.0),
            store,
        }),
        stencil_ops: two.then_some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(0),
            store: wgpu::StoreOp::Discard,
        }),
    }
}

/// Whether the scene passes on `device` write their depth in the fragment
/// shader (`scene.wgsl`, `depth_key`) instead of taking the rasteriser's.
///
/// They do on the OpenGL backend, whose clip space has its depth in
/// -1..1 where WebGPU's has 0..1: wgpu's shader translation converts the
/// vertex stage's `z` by `z * 2 - w` in `f32`, and the hardware maps the
/// result back to 0..1. The reversed key `near / dist` is tiny far away
/// (a near plane right in front of the eye, as a camera inside a model
/// needs, makes it near / far, 1e-6 at the least), and a number that
/// small does not survive the trip: whatever the key, the depth that
/// comes out of the conversion is only exact to about 2⁻²⁵ in 0..1 (the
/// spacing of an `f32` next to 1), which at `near / far` of 1e-6 is 3 %
/// of the key, or a metre at 43 m (measured on a Radeon 8060S: every
/// pixel of a building 1 m nearer than its own surface; on an Intel MTL
/// up to 0.6 m, 2 cm at 30 m). Writing the key from the interpolated
/// distance in the fragment shader is exact to the `f32` division on
/// every GPU; it costs the early depth test, so only the backend whose
/// conversion loses the key pays it. docs/design/rendering.md
/// (2026-10-19) has the measurements.
pub(crate) fn fragment_depth(device: &wgpu::Device) -> bool {
    device.adapter_info().backend == wgpu::Backend::Gl
}

/// `scene.wgsl` followed by `more`, with its template filled in for
/// `device` ([`fragment_depth`]) and one sample per pixel.
pub(crate) fn scene_source(device: &wgpu::Device, more: &[&str]) -> String {
    scene_source_for(fragment_depth(device), false, false, more)
}

/// The front's depth at the fragment's pixel, loaded from the depth buffer
/// of the front (`peel.wgsl`).
const LOADED_FRONT: &str = "textureLoad(front_depth, vec2<i32>(in.clip.xy), 0).r";

/// The same texel, sampled at its centre through the nearest sampler of
/// [`FRONT_SAMPLER`]: what OpenGL ES 3.0 (WebGL2) reads a depth texture
/// with, since it takes one whose filter is not the nearest texel for
/// incomplete and loads 0 from it (`peel::sampled_front`). The lines'
/// distance pass reads the depth buffer the same way, for the same reason
/// (`onscreen::SAMPLED_DEPTH`, with the texel's coordinates already
/// integers there); keep the two alike.
const SAMPLED_FRONT: &str = "textureSampleLevel(front_depth, front_sampler, \
     (floor(in.clip.xy) + vec2<f32>(0.5)) / vec2<f32>(textureDimensions(front_depth)), 0.0).r";

/// The declaration of that sampler, at `peel::FRONT_SAMPLER_BINDING`.
const FRONT_SAMPLER: &str = "@group(1) @binding(2) var front_sampler: sampler;";

/// [`scene_source`] for a backend that does (`fragment_depth`) or does
/// not write the depth in the fragment shader, and for a renderer with
/// several samples per pixel (`multisampled`: a drawn fragment's cut is
/// then decided at its centroid, and with `per_sample`, on a device that
/// shades per sample, the count of a section is per sample; `cut.wgsl`)
/// or with one. Every variant of every composition is checked by the
/// tests below, so that a machine without an OpenGL adapter still
/// validates the OpenGL shaders, and one with per-sample shading the
/// composition for a device without.
pub(crate) fn scene_source_for(
    fragment_depth: bool,
    multisampled: bool,
    per_sample: bool,
    more: &[&str],
) -> String {
    let mut source = include_str!("scene.wgsl").to_string();
    for part in more {
        source.push('\n');
        source.push_str(part);
    }
    let (declare, set, stored) = if fragment_depth {
        (
            "@builtin(frag_depth) depth: f32,",
            "out.depth = depth_key(in.dist);",
            "depth_key(dist)",
        )
    } else {
        ("", "", "clip_z")
    };
    let centroid = if multisampled {
        "@interpolate(perspective, centroid)"
    } else {
        ""
    };
    let sample = if multisampled && per_sample {
        "@interpolate(perspective, sample)"
    } else {
        ""
    };
    // How the pass behind the front reads the front's depth buffer
    // (`peel.wgsl`, `cut_flat.wgsl`): on OpenGL, the backend that writes
    // the depth in the fragment shader, through a sampler that takes the
    // nearest texel (`peel::sampled_front`).
    let (front_sampler, front_depth) = if fragment_depth {
        (FRONT_SAMPLER, SAMPLED_FRONT)
    } else {
        ("", LOADED_FRONT)
    };
    source
        .replace("{{FRAG_DEPTH}}", declare)
        .replace("{{SET_DEPTH}}", set)
        .replace("{{STORED_DEPTH}}", stored)
        .replace("{{FRONT_SAMPLER}}", front_sampler)
        .replace("{{FRONT_DEPTH}}", front_depth)
        .replace("{{SIDE_SAMPLE}}", sample)
        .replace("{{SIDE_CENTROID}}", centroid)
}

/// The state every pass over the triangles shares: both sides of every
/// triangle (sheets are seen from either side, as on the CPU), and
/// `Greater` (the depth is reversed, `scene.wgsl`), so that on a tie the
/// triangle drawn first stays.
pub(crate) fn scene_primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        cull_mode: None,
        ..wgpu::PrimitiveState::default()
    }
}

/// A pipeline over the triangles of a mesh with the vertex stage of
/// `scene.wgsl`, in `step`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    entry: &str,
    groups: &[Option<&wgpu::BindGroupLayout>],
    targets: &[Option<wgpu::ColorTargetState>],
    samples: u32,
    step: Step,
) -> wgpu::RenderPipeline {
    scene_pipeline_from(
        device,
        label,
        module,
        ("vs_main", entry),
        groups,
        targets,
        samples,
        step,
    )
}

/// [`scene_pipeline`] with the vertex entry `vertex` (`vs_studio` for
/// the studio look's jitter).
#[allow(clippy::too_many_arguments)]
pub(crate) fn scene_pipeline_from(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    (vertex, entry): (&str, &str),
    groups: &[Option<&wgpu::BindGroupLayout>],
    targets: &[Option<wgpu::ColorTargetState>],
    samples: u32,
    step: Step,
) -> wgpu::RenderPipeline {
    let targets = step.targets(targets);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: groups,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &crate::mesh::mesh_buffers(&MESH_ATTRIBUTES),
        },
        primitive: scene_primitive(),
        depth_stencil: Some(step.depth_stencil()),
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl SeeThrough {
    fn build(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout, two: bool) -> SeeThrough {
        let module = peel::scene_module(device, false);
        let peel_layout = peel::peel_layout(device, false);
        let blend_layout = peel::blend_layout(device, false, true);
        let front = Drawing::build(two, |step| {
            scene_pipeline(
                device,
                "craft3d front",
                &module,
                "fs_front",
                &[Some(scene_layout)],
                &[target(COLOR_FORMAT), target(DATA_FORMAT)],
                1,
                step,
            )
        });
        let behind = Drawing::build(two, |step| {
            scene_pipeline(
                device,
                "craft3d behind",
                &module,
                "fs_behind",
                &[Some(scene_layout), Some(&peel_layout)],
                &[target(COLOR_FORMAT)],
                1,
                step,
            )
        });
        let blend_pipeline = peel::blend_pipeline(
            device,
            &peel::blend_module(device, false),
            &blend_layout,
            true,
        );
        SeeThrough {
            front,
            behind,
            blend_pipeline,
            peel_layout,
            blend_layout,
            table: peel::Table::new(device),
            front_sampler: peel::front_sampler(device),
        }
    }
}

pub(crate) fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(crate) fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(crate) fn target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })
}

const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };

impl Renderer {
    /// Builds the pipelines on `device`. Nothing is drawn or allocated for
    /// an image until the first [`Renderer::draw`].
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Renderer, GpuError> {
        Self::with_steps(device, queue, device.features().contains(TWO_STEPS_FEATURE))
    }

    /// [`Renderer::new`], in two steps ([`Step`]) or in one.
    fn with_steps(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        two: bool,
    ) -> Result<Renderer, GpuError> {
        let mut renderer = guarded(device, || Self::build(device, queue, two))?;
        // On its own: a device that cannot do this still draws opaque
        // pictures, and says why when asked for a see-through one.
        renderer.see_through = guarded(device, || {
            SeeThrough::build(device, &renderer.scene_layout, two)
        })
        .map_err(|e| e.to_string());
        Ok(renderer)
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
                CutPipelines::build(&self.device, &self.scene_layout, peel_layout, self.two)
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

    /// Draws every pass over the triangles in one step, as on a device
    /// without 32-bit float depth with a stencil ([`Step`]): the renderer
    /// is made again, and meshes uploaded before are its own still. For
    /// the test that both ways give the same picture, and for measuring
    /// the Radeon's lost writes.
    #[doc(hidden)]
    pub fn in_one_step(&mut self) -> Result<(), GpuError> {
        let mut again = Self::with_steps(&self.device, &self.queue, false)?;
        again.token = Arc::clone(&self.token);
        *self = again;
        Ok(())
    }

    /// Whether the passes over the triangles are drawn in two steps
    /// ([`Step`]).
    #[doc(hidden)]
    pub fn in_two_steps(&self) -> bool {
        self.two
    }

    fn build(device: &wgpu::Device, queue: &wgpu::Queue, two: bool) -> Renderer {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d scene"),
            source: wgpu::ShaderSource::Wgsl(scene_source(device, &[]).into()),
        });
        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("resolve.wgsl").into()),
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d present"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });

        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d scene"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
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
        // Both sides of every triangle are drawn (`scene_primitive`):
        // sheets are seen from either side, as on the CPU.
        let scene = Drawing::build(two, |step| {
            scene_pipeline(
                device,
                "craft3d scene",
                &scene_shader,
                "fs_main",
                &[Some(&scene_layout)],
                &[target(COLOR_FORMAT), target(DATA_FORMAT)],
                1,
                step,
            )
        });

        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d resolve"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, UNFILTERED),
                texture_entry(2, wgpu::TextureSampleType::Uint),
            ],
        });
        let resolve_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d resolve uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let resolve_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d resolve"),
                bind_group_layouts: &[Some(&resolve_layout)],
                immediate_size: 0,
            });
        let resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d resolve"),
            layout: Some(&resolve_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("fs_resolve"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[target(COLOR_FORMAT), target(DATA_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
        });

        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d present"),
            entries: &[texture_entry(0, UNFILTERED)],
        });

        Renderer {
            device: device.clone(),
            queue: queue.clone(),
            two,
            scene,
            scene_uniforms,
            scene_layout,
            scene_bind,
            resolve_pipeline,
            resolve_layout,
            resolve_uniforms,
            present_shader,
            present_layout,
            present_pipelines: Vec::new(),
            targets: None,
            see_through: Err("it was not set up".to_string()),
            cut: None,
            cap_buffer: cut::cap_buffer(device),
            studio: None,
            staging: Mutex::new(None),
            drawn: false,
            token: Arc::new(()),
        }
    }

    /// The device this renderer draws on.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Puts `mesh` on the GPU. An inconsistent mesh is the CPU renderer's
    /// [`RenderError::InvalidMesh`].
    pub fn upload(&self, mesh: &Mesh) -> Result<GpuMesh, GpuError> {
        self.upload_colored(mesh, &FaceColors::new())
    }

    /// [`Self::upload_colored`] with a value per vertex for the curvature
    /// map (`craft3d_render::render_analysed`).
    pub fn upload_analysed(
        &self,
        mesh: &Mesh,
        colors: &FaceColors,
        values: &[f32],
    ) -> Result<GpuMesh, GpuError> {
        guarded(&self.device, || {
            GpuMesh::upload(&self.device, &self.queue, mesh, colors, values, &self.token)
        })?
    }

    /// [`Self::upload`] with faces in colours of their own
    /// (`craft3d_render::render_colored`): the colours go with the mesh, so
    /// a change of colour is a new upload.
    pub fn upload_colored(&self, mesh: &Mesh, colors: &FaceColors) -> Result<GpuMesh, GpuError> {
        guarded(&self.device, || {
            GpuMesh::upload(&self.device, &self.queue, mesh, colors, &[], &self.token)
        })?
    }

    /// Puts a scene's bodies on the GPU each in its own coordinates, each
    /// drawn with its pose ([`MeshPart`]): what an assembly's moments
    /// move with [`Self::set_poses`] instead of uploading their meshes
    /// again. `bounds` is the box of the triangles as placed
    /// ([`crate::drawn_bounds`] of the scene's mesh): the depth range, the
    /// section and the studio light are framed on it, as the CPU renderer
    /// frames them on the placed mesh. Face ids and colours are the
    /// scene's (each part's from its `first_face_id` on).
    pub fn upload_parts(
        &self,
        parts: &[MeshPart<'_>],
        colors: &FaceColors,
        bounds: Bbox3,
    ) -> Result<GpuMesh, GpuError> {
        guarded(&self.device, || {
            GpuMesh::upload_parts(
                &self.device,
                &self.queue,
                parts,
                colors,
                bounds,
                &self.token,
            )
        })?
    }

    /// [`Self::upload_parts`] taking the uploaded vertices of the parts of
    /// `old` that `reuse` names for each new part, by its place in `old`
    /// (`Onscreen::upload_parts_from`).
    pub fn upload_parts_from(
        &self,
        parts: &[MeshPart<'_>],
        colors: &FaceColors,
        bounds: Bbox3,
        old: Option<GpuMesh>,
        reuse: &[Option<usize>],
    ) -> Result<GpuMesh, GpuError> {
        let old = old.filter(|o| Arc::ptr_eq(&o.owner, &self.token));
        guarded(&self.device, || {
            GpuMesh::upload_parts_from(
                &self.device,
                &self.queue,
                parts,
                colors,
                bounds,
                &self.token,
                old,
                reuse,
            )
        })?
    }

    /// Moves the parts of a mesh uploaded by [`Self::upload_parts`] to
    /// `poses` (one per part, in order) with `bounds` the box of the
    /// triangles as now placed: a few matrices are written, no geometry.
    /// A mesh of another renderer, or a count of poses that is not its
    /// count of parts, is refused.
    pub fn set_poses(
        &self,
        mesh: &mut GpuMesh,
        poses: &[Transform],
        bounds: Bbox3,
    ) -> Result<(), GpuError> {
        if !Arc::ptr_eq(&mesh.owner, &self.token) {
            return Err(GpuError::Device(
                "the mesh was uploaded by another renderer".to_string(),
            ));
        }
        guarded(&self.device, || {
            mesh.set_poses(&self.device, &self.queue, poses, bounds)
        })?
    }

    /// Renders `mesh` as seen from `camera` into the renderer's own targets.
    /// Afterwards [`Renderer::color_view`] is the picture and
    /// [`Renderer::read_frame`] reads it back with ids and distances.
    pub fn draw(
        &mut self,
        mesh: &GpuMesh,
        camera: &Camera,
        params: &RenderParams,
    ) -> Result<(), GpuError> {
        self.draw_and_present(mesh, camera, params, None)
    }

    /// [`Renderer::draw`], then a copy of the picture into `target`, a
    /// colour view of the given `format` on this renderer's device, which
    /// the caller owns. The copy is
    /// pixel for pixel from the top-left corner, so `target` should be
    /// `params.width` × `params.height`; a larger one is cleared to black
    /// first and repeats the picture's last row and column.
    pub fn draw_into(
        &mut self,
        mesh: &GpuMesh,
        camera: &Camera,
        params: &RenderParams,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<(), GpuError> {
        self.present_pipeline(format)?;
        self.draw_and_present(mesh, camera, params, Some((target, format)))
    }

    /// The resolved picture (`Rgba8Unorm`, `params.width` × `params.height`)
    /// of the last successful draw, for a shell that shows it as a texture.
    /// It is replaced when the image size changes.
    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        let targets = self.targets.as_ref().filter(|_| self.drawn)?;
        Some(&targets.color_view)
    }

    /// Width and height of the last successful draw.
    pub fn size(&self) -> Option<(u32, u32)> {
        let targets = self.targets.as_ref().filter(|_| self.drawn)?;
        Some((targets.width, targets.height))
    }

    /// Reads the last draw back as the CPU renderer's [`Frame`]. Blocks
    /// until the GPU is done.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_frame(&self) -> Result<Frame, GpuError> {
        let pending = self.start_readback()?;
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        pending.finish()
    }

    /// [`Renderer::read_frame`] for callers that cannot block (the browser).
    pub async fn read_frame_async(&self) -> Result<Frame, GpuError> {
        let pending = self.start_readback()?;
        // Native wgpu maps buffers only when polled; in the browser the
        // event loop does it.
        #[cfg(not(target_arch = "wasm32"))]
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        pending.mapped().await;
        pending.finish()
    }

    fn start_readback(&self) -> Result<readback::Pending, GpuError> {
        let targets = self
            .targets
            .as_ref()
            .filter(|_| self.drawn)
            .ok_or(GpuError::NothingDrawn)?;
        let mut staging = self.staging.lock().unwrap_or_else(PoisonError::into_inner);
        guarded(&self.device, || {
            readback::start(
                &self.device,
                &self.queue,
                (&targets.color, Some(&targets.data)),
                (targets.width, targets.height),
                // The first of the pixel's ssaa × ssaa samples.
                0.5 / f64::from(targets.ssaa.max(1)),
                &mut staging,
            )
        })?
    }

    /// The pipeline that copies into a target of `format`, made on first use.
    fn present_pipeline(&mut self, format: wgpu::TextureFormat) -> Result<(), GpuError> {
        if self.present_pipelines.iter().any(|(f, _)| *f == format) {
            return Ok(());
        }
        let pipeline = guarded(&self.device, || {
            let layout = self
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("craft3d present"),
                    bind_group_layouts: &[Some(&self.present_layout)],
                    immediate_size: 0,
                });
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("craft3d present"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.present_shader,
                        entry_point: Some("vs_fullscreen"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.present_shader,
                        // An sRGB target encodes on write; the picture is
                        // already display values.
                        entry_point: Some(if format.is_srgb() {
                            "fs_present_srgb"
                        } else {
                            "fs_present"
                        }),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[target(format)],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        self.present_pipelines.push((format, pipeline));
        Ok(())
    }

    fn draw_and_present(
        &mut self,
        mesh: &GpuMesh,
        camera: &Camera,
        params: &RenderParams,
        present: Option<(&wgpu::TextureView, wgpu::TextureFormat)>,
    ) -> Result<(), GpuError> {
        // Whatever goes wrong below, the previous frame is not this one.
        self.drawn = false;
        // wgpu cannot tell a buffer of another instance from one of its own
        // (it panics, or draws a different buffer), so we must.
        if !Arc::ptr_eq(&mesh.owner, &self.token) {
            return Err(GpuError::ForeignMesh);
        }
        check_params(params, self.device.limits().max_texture_dimension_2d)?;
        let see_through = params.opacity() < 1.0;
        if see_through && let Err(why) = &self.see_through {
            return Err(GpuError::SeeThrough(why.clone()));
        }
        let cut = cut::frame_cut(mesh, params);
        if cut.is_some() {
            self.ensure_cut()?;
        }
        let scene = scene_uniforms(mesh, camera, params, cut.as_ref())?;
        let device = self.device.clone();
        guarded(&device, || {
            self.ensure_targets(params, see_through);
            if cut.is_some() {
                self.ensure_cut_targets(see_through);
            }
            self.encode(mesh, &scene, params, cut.as_ref(), present)
        })??;
        self.drawn = true;
        Ok(())
    }

    fn ensure_targets(&mut self, params: &RenderParams, see_through: bool) {
        self.ensure_frame_targets(params);
        if !see_through {
            return;
        }
        let (Some(targets), Ok(pipelines)) = (&mut self.targets, &self.see_through) else {
            return;
        };
        if targets.peel.is_some() {
            return;
        }
        let depth_format =
            Step::depth_stencil(if self.two { Step::Depth } else { Step::One }).format;
        let device = &self.device;
        let texture = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: targets.width.saturating_mul(targets.ssaa),
                    height: targets.height.saturating_mul(targets.ssaa),
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
        let behind = view(&texture(
            "craft3d samples behind",
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        let depth = view(&texture(
            "craft3d samples depth behind",
            depth_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        let peel_bind = peel::peel_bind(
            device,
            &pipelines.peel_layout,
            false,
            &targets.samples_depth_read,
            &pipelines.front_sampler,
        );
        let bound = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let blend_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d blend"),
            layout: &pipelines.blend_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.resolve_uniforms.as_entire_binding(),
                },
                bound(1, &targets.samples_color),
                bound(2, &behind),
                bound(3, &pipelines.table.view),
                bound(4, &targets.samples_data),
            ],
        });
        targets.peel = Some(PeelTargets {
            behind,
            depth,
            peel_bind,
            blend_bind,
        });
    }

    /// The count of a cut-open picture, at the size of the samples, and
    /// for a see-through one the group of the cap behind the front.
    fn ensure_cut_targets(&mut self, see_through: bool) {
        let (Some(targets), Some(Ok(pipelines))) = (&mut self.targets, &self.cut) else {
            return;
        };
        let device = &self.device;
        if targets.cut.is_none() {
            let parity = cut::parity_view(
                device,
                targets.width.saturating_mul(targets.ssaa),
                targets.height.saturating_mul(targets.ssaa),
                1,
            );
            let cap_bind = cut::cap_bind(device, &pipelines.cap_layout, false, None, &parity);
            targets.cut = Some(CutTargets {
                parity,
                cap_bind,
                cap_behind_bind: None,
            });
        }
        if see_through
            && let (Some(cut), Some(peeled), Ok(peel)) =
                (&mut targets.cut, &pipelines.peeled, &self.see_through)
            && cut.cap_behind_bind.is_none()
        {
            cut.cap_behind_bind = Some(cut::cap_bind(
                device,
                &peeled.cap_behind_layout,
                false,
                Some((&targets.samples_depth_read, &peel.front_sampler)),
                &cut.parity,
            ));
        }
    }

    fn ensure_frame_targets(&mut self, params: &RenderParams) {
        let (width, height, ssaa) = (params.width, params.height, params.ssaa);
        if self
            .targets
            .as_ref()
            .is_some_and(|t| (t.width, t.height, t.ssaa) == (width, height, ssaa))
        {
            return;
        }
        // Drop the old textures before allocating the new ones.
        self.targets = None;
        let device = &self.device;
        let texture = |label: &str, format, scale: u32, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                // Within the device limit: check_params ran first.
                size: wgpu::Extent3d {
                    width: width.saturating_mul(scale),
                    height: height.saturating_mul(scale),
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
        let sampled = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let read = sampled | wgpu::TextureUsages::COPY_SRC;
        let samples_color = view(&texture(
            "craft3d samples colour",
            COLOR_FORMAT,
            ssaa,
            sampled,
        ));
        let samples_data = view(&texture("craft3d samples data", DATA_FORMAT, ssaa, sampled));
        // Read by the pass behind the front of a see-through picture.
        let depth_texture = texture(
            "craft3d samples depth",
            Step::depth_stencil(if self.two { Step::Depth } else { Step::One }).format,
            ssaa,
            sampled,
        );
        let samples_depth = view(&depth_texture);
        let samples_depth_read = depth_texture.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..wgpu::TextureViewDescriptor::default()
        });
        let color = texture("craft3d colour", COLOR_FORMAT, 1, read);
        let data = texture("craft3d data", DATA_FORMAT, 1, read);
        let (color_view, data_view) = (view(&color), view(&data));
        let resolve_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d resolve"),
            layout: &self.resolve_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.resolve_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&samples_color),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&samples_data),
                },
            ],
        });
        let present_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d present"),
            layout: &self.present_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            }],
        });
        self.targets = Some(Targets {
            width,
            height,
            ssaa,
            samples_color,
            samples_data,
            samples_depth,
            samples_depth_read,
            color,
            data,
            color_view,
            data_view,
            resolve_bind,
            present_bind,
            peel: None,
            cut: None,
        });
    }

    fn encode(
        &self,
        mesh: &GpuMesh,
        scene: &[f32; SCENE_FLOATS],
        params: &RenderParams,
        cut: Option<&Cut>,
        present: Option<(&wgpu::TextureView, wgpu::TextureFormat)>,
    ) -> Result<(), GpuError> {
        let targets = self.targets.as_ref().ok_or(GpuError::NothingDrawn)?;
        self.queue
            .write_buffer(&self.scene_uniforms, 0, &float_bytes(scene));
        let resolve: Vec<u8> = [params.ssaa, 0, 0, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.queue.write_buffer(&self.resolve_uniforms, 0, &resolve);

        let [r, g, b] = params.background.map(|c| f64::from(c) / 255.0);
        let cleared = |view, color| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        // Where nothing is drawn: no face, at an infinite distance, as in
        // the CPU renderer's frame.
        let nothing = wgpu::Color {
            r: f64::from(NO_FACE),
            g: f64::from(f32::INFINITY.to_bits()),
            b: 0.0,
            a: 0.0,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("craft3d frame"),
            });
        let opacity = params.opacity();
        // The passes of a see-through picture, when it is one.
        let peeled = if opacity < 1.0 {
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
            mesh.draw_in(pass);
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
        if let Some((pipelines, count, true)) = cut_open {
            cut::count_pass(
                &mut encoder,
                &count.parity,
                &pipelines.parity,
                &self.scene_bind,
                mesh,
            );
        }
        let two = self.two;
        let mesh_draw = |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
            pass.set_pipeline(pipeline);
            draw_mesh(pass);
        };
        {
            // The front. Seen through, its alpha says where (0 where
            // nothing is drawn) and its depth is kept for the next pass.
            let alpha = if peeled.is_some() { 0.0 } else { 1.0 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d scene"),
                color_attachments: &[
                    cleared(&targets.samples_color, wgpu::Color { r, g, b, a: alpha }),
                    cleared(&targets.samples_data, nothing),
                ],
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.samples_depth,
                    two,
                    peeled.is_some(),
                )),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let front = match (cut_open, peeled) {
                (Some((cut, ..)), Some(_)) => {
                    let peeled = cut.peeled.as_ref().ok_or(GpuError::NothingDrawn)?;
                    &peeled.front
                }
                (Some((cut, ..)), None) => &cut.scene,
                (None, Some((pipelines, _))) => &pipelines.front,
                (None, None) => &self.scene,
            };
            pass.set_bind_group(0, &self.scene_bind, &[]);
            let cap_draw = |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
                if let Some((_, count, true)) = cut_open {
                    cut::draw_cap(pass, pipeline, &count.cap_bind, &self.cap_buffer, mesh);
                }
            };
            match cut_open {
                Some((pipelines, _, true)) => draw_in_steps(
                    &mut pass,
                    &[(front, &mesh_draw), (&pipelines.cap, &cap_draw)],
                ),
                _ => draw_in_steps(&mut pass, &[(front, &mesh_draw)]),
            }
        }
        if let Some((pipelines, peel)) = peeled {
            // The surface behind the front one.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d behind"),
                color_attachments: &[cleared(&peel.behind, wgpu::Color { r, g, b, a: 1.0 })],
                depth_stencil_attachment: Some(depth_attachment(&peel.depth, two, false)),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let cut_peeled = match cut_open {
                Some((cut, ..)) => Some(cut.peeled.as_ref().ok_or(GpuError::NothingDrawn)?),
                None => None,
            };
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, &peel.peel_bind, &[]);
            let behind = cut_peeled.map_or(&pipelines.behind, |cut| &cut.behind);
            match (cut_peeled, cut_open) {
                (Some(cut_peeled), Some((_, count, true))) => {
                    let bind = count
                        .cap_behind_bind
                        .as_ref()
                        .ok_or(GpuError::NothingDrawn)?;
                    let peel_bind = &peel.peel_bind;
                    // The mesh with the front's depth bound, the cap with
                    // its own group 1.
                    let mesh_behind =
                        |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
                            pass.set_bind_group(1, peel_bind, &[]);
                            pass.set_pipeline(pipeline);
                            draw_mesh(pass);
                        };
                    let cap_behind =
                        |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
                            cut::draw_cap(pass, pipeline, bind, &self.cap_buffer, mesh);
                        };
                    draw_in_steps(
                        &mut pass,
                        &[
                            (behind, &mesh_behind),
                            (&cut_peeled.cap_behind, &cap_behind),
                        ],
                    );
                }
                _ => draw_in_steps(&mut pass, &[(behind, &mesh_draw)]),
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d resolve"),
                color_attachments: &[
                    cleared(&targets.color_view, wgpu::Color::BLACK),
                    cleared(&targets.data_view, nothing),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            match peeled {
                Some((pipelines, peel)) => {
                    pass.set_pipeline(&pipelines.blend_pipeline);
                    pass.set_bind_group(0, &peel.blend_bind, &[]);
                }
                None => {
                    pass.set_pipeline(&self.resolve_pipeline);
                    pass.set_bind_group(0, &targets.resolve_bind, &[]);
                }
            }
            pass.draw(0..3, 0..1);
        }
        if let Some((view, format)) = present {
            let pipeline = self
                .present_pipelines
                .iter()
                .find(|(f, _)| *f == format)
                .map(|(_, p)| p)
                .ok_or_else(|| GpuError::Device("no pipeline for the target format".to_string()))?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d present"),
                color_attachments: &[cleared(view, wgpu::Color::BLACK)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &targets.present_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

/// The studio look of the readback renderer: the shared parts, the
/// geometry pass and the surfaces at one sample per target sample, and the
/// background, with the per-size targets.
struct StudioReadback {
    gpu: StudioGpu,
    geometry: Drawing,
    background: wgpu::RenderPipeline,
    main: Drawing,
    /// The surfaces' light and weight (the screen-space buffers), and
    /// their face ids and distances in a pass of their own.
    main_post: Drawing,
    data: Drawing,
    /// Layer 3: the geometry and the light of the picture without the
    /// faces that let light through, and their back faces.
    geometry_behind: Drawing,
    main_behind: Drawing,
    backs: Drawing,
    /// A see-through picture: the geometry and the light of the surface
    /// behind the front.
    geometry_peel: Drawing,
    main_peel: Drawing,
    targets: Option<StudioTargets>,
    through: Option<studio::ThroughTargets>,
    polish: Option<studio::PolishTargets>,
    /// The sum of a picture's frames, at its pixels.
    accumulator: Option<studio::Accumulator>,
}

impl StudioReadback {
    fn build(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout, two: bool) -> Self {
        let module = studio::studio_module(device);
        let gpu = StudioGpu::new(device, &module, scene_layout);
        let geometry = Drawing::build(two, |step| {
            scene_pipeline_from(
                device,
                "craft3d geometry",
                &module,
                ("vs_studio", "fs_geometry"),
                &[
                    Some(scene_layout),
                    Some(&gpu.layout_uniform),
                    Some(&gpu.layout_parity),
                ],
                &[target(studio::GEOMETRY_FORMAT)],
                1,
                step,
            )
        });
        let main = Drawing::build(two, |step| {
            scene_pipeline_from(
                device,
                "craft3d studio scene",
                &module,
                ("vs_studio", "fs_studio"),
                &[Some(scene_layout), Some(&gpu.layout_main)],
                &[target(COLOR_FORMAT), target(DATA_FORMAT)],
                1,
                step,
            )
        });
        let main_post = Drawing::build(two, |step| {
            scene_pipeline_from(
                device,
                "craft3d studio scene post",
                &module,
                ("vs_studio", "fs_studio_post"),
                &[
                    Some(scene_layout),
                    Some(&gpu.layout_main),
                    Some(&gpu.layout_parity),
                ],
                &[target(studio::LIGHT_FORMAT), target(studio::LIGHT_FORMAT)],
                1,
                step,
            )
        });
        let data = Drawing::build(two, |step| {
            scene_pipeline_from(
                device,
                "craft3d studio data",
                &module,
                ("vs_studio", "fs_studio_data"),
                &[
                    Some(scene_layout),
                    Some(&gpu.layout_uniform),
                    Some(&gpu.layout_parity),
                ],
                &[target(DATA_FORMAT)],
                1,
                step,
            )
        });
        let mesh_drawing = |label: &str,
                            entry: &str,
                            group: &wgpu::BindGroupLayout,
                            format: wgpu::TextureFormat,
                            count: usize| {
            Drawing::build(two, |step| {
                scene_pipeline_from(
                    device,
                    label,
                    &module,
                    ("vs_studio", entry),
                    &[Some(scene_layout), Some(group), Some(&gpu.layout_parity)],
                    &vec![target(format); count],
                    1,
                    step,
                )
            })
        };
        let geometry_behind = mesh_drawing(
            "craft3d geometry behind",
            "fs_geometry_opaque",
            &gpu.layout_uniform,
            studio::GEOMETRY_FORMAT,
            1,
        );
        let main_behind = mesh_drawing(
            "craft3d studio scene behind",
            "fs_studio_behind",
            &gpu.layout_main,
            studio::LIGHT_FORMAT,
            2,
        );
        let backs = mesh_drawing(
            "craft3d back faces",
            "fs_backs",
            &gpu.layout_uniform,
            studio::BITS_FORMAT,
            1,
        );
        let geometry_peel = mesh_drawing(
            "craft3d geometry peeled",
            "fs_geometry_peel",
            &gpu.layout_ao,
            studio::GEOMETRY_FORMAT,
            1,
        );
        let main_peel = mesh_drawing(
            "craft3d studio scene peeled",
            "fs_studio_peel",
            &gpu.layout_main,
            studio::LIGHT_FORMAT,
            2,
        );
        let background = studio::fullscreen_pipeline(
            device,
            "craft3d background",
            &module,
            "fs_background",
            &[Some(scene_layout), Some(&gpu.layout_main)],
            COLOR_FORMAT,
            1,
        );
        StudioReadback {
            geometry_peel,
            main_peel,
            geometry_behind,
            main_behind,
            backs,
            through: None,
            gpu,
            geometry,
            background,
            main,
            main_post,
            data,
            targets: None,
            polish: None,
            accumulator: None,
        }
    }
}

/// Where a frame stands in the accumulation of a still picture.
#[derive(Clone, Copy)]
struct InSum {
    /// The first frame drawn: it starts the sum and draws the shadow map.
    first: bool,
    /// The last: the mean goes into the picture.
    last: bool,
    /// Whether there is a sum at all (more than one frame).
    summed: bool,
}

impl Renderer {
    /// [`Renderer::draw`] in the studio look (`craft3d_render::studio`,
    /// `render_studio`): lit by `studio`'s environment, with its shadows,
    /// occlusion and ground, and layer 2's effects; the mean of
    /// `Look::accumulate` jittered frames, summed per pixel and rounded
    /// once, with the face ids and depths of frame 0. A see-through or
    /// cut-open picture as the CPU renderer defines it (`render_studio`):
    /// the surface behind the front drawn first and the front blended
    /// over it, the section's count drawn first and its cap with every
    /// pass of the surfaces.
    pub fn draw_studio(
        &mut self,
        mesh: &GpuMesh,
        camera: &Camera,
        params: &RenderParams,
        studio: &Studio,
    ) -> Result<(), GpuError> {
        self.drawn = false;
        if !Arc::ptr_eq(&mesh.owner, &self.token) {
            return Err(GpuError::ForeignMesh);
        }
        check_params(params, self.device.limits().max_texture_dimension_2d)?;
        let cut = cut::frame_cut(mesh, params);
        let opened = (params.opacity(), cut.is_some());
        if self.studio.is_none() {
            let (device, layout, two) = (self.device.clone(), &self.scene_layout, self.two);
            self.studio = Some(
                guarded(&device, || StudioReadback::build(&device, layout, two))
                    .map_err(|e| e.to_string()),
            );
        }
        let scene = scene_uniforms(mesh, camera, params, cut.as_ref())?;
        let size = (
            params.width.saturating_mul(params.ssaa),
            params.height.saturating_mul(params.ssaa),
        );
        let (_, first_passes) =
            studio::studio_uniforms(mesh, studio, camera, size, 0, params.background, opened)?;
        // The cap's triangle, the same for every frame.
        let cap = cut.as_ref().and_then(|cut| cut::cap_vertices(mesh, cut));
        if let Some(bytes) = &cap {
            self.queue.write_buffer(&self.cap_buffer, 0, bytes);
        }
        let cap = cap.is_some();
        let frames = first_passes.frames;
        // Frame 0 last, so that the ids and depths are its own; the sum
        // does not depend on the order.
        let order = (1..frames).chain(std::iter::once(0));
        for (i, frame) in order.enumerate() {
            let (light, passes) = studio::studio_uniforms(
                mesh,
                studio,
                camera,
                size,
                frame,
                params.background,
                opened,
            )?;
            let place = InSum {
                first: i == 0,
                last: frame == 0,
                summed: frames > 1,
            };
            let device = self.device.clone();
            guarded(&device, || {
                self.ensure_frame_targets(params);
                let queue = self.queue.clone();
                let look = match &mut self.studio {
                    Some(Ok(look)) => look,
                    Some(Err(why)) => {
                        return Err(GpuError::Device(format!(
                            "the studio look cannot be drawn on this device: {why}"
                        )));
                    }
                    None => return Err(GpuError::NothingDrawn),
                };
                look.gpu.ensure_env(&device, &queue, &studio.env)?;
                if look.targets.as_ref().is_none_or(|t| t.size != size) {
                    look.targets = None;
                    look.targets =
                        Some(StudioTargets::new(&device, size).counted(&look.gpu, &device));
                }
                if !passes.transmit && !passes.peel {
                    look.through = None;
                } else if look.through.as_ref().is_none_or(|t| t.size != size) {
                    look.through = None;
                    look.through = Some(studio::ThroughTargets::new(&device, size));
                }
                if passes.post
                    && look
                        .polish
                        .as_ref()
                        .is_none_or(|t| !t.fits(size, &passes.levels))
                {
                    look.polish = None;
                    look.polish = Some(studio::PolishTargets::new(&device, size, &passes.levels));
                }
                let pixels = (params.width, params.height);
                if place.summed && look.accumulator.as_ref().is_none_or(|a| a.size != pixels) {
                    look.accumulator = None;
                    look.accumulator = Some(studio::Accumulator::new(&device, pixels));
                }
                self.encode_studio(mesh, &scene, &light, &passes, params, (place, cap))
            })??;
        }
        self.drawn = true;
        Ok(())
    }

    fn encode_studio(
        &mut self,
        mesh: &GpuMesh,
        scene: &[f32; SCENE_FLOATS],
        light: &[f32; studio::STUDIO_FLOATS],
        passes: &Passes,
        params: &RenderParams,
        (place, cap): (InSum, bool),
    ) -> Result<(), GpuError> {
        let targets = self.targets.as_ref().ok_or(GpuError::NothingDrawn)?;
        let Some(Ok(look)) = &mut self.studio else {
            return Err(GpuError::NothingDrawn);
        };
        let sized = look.targets.as_ref().ok_or(GpuError::NothingDrawn)?;
        self.queue
            .write_buffer(&self.scene_uniforms, 0, &float_bytes(scene));
        self.queue
            .write_buffer(&look.gpu.uniforms, 0, &float_bytes(light));
        let resolve: Vec<u8> = [params.ssaa, 0, 0, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.queue.write_buffer(&self.resolve_uniforms, 0, &resolve);
        let attachment = |view, load| {
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
        let nothing = wgpu::Color {
            r: f64::from(NO_FACE),
            g: f64::from(f32::INFINITY.to_bits()),
            b: 0.0,
            a: 0.0,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("craft3d studio frame"),
            });
        let cap_buffer = &self.cap_buffer;
        // The mesh and, in a cut-open picture, the cap's triangle (its
        // corners relative to the mesh's origin: the identity pose).
        let draw_mesh = |pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline| {
            pass.set_pipeline(pipeline);
            mesh.draw_in(pass);
            if cap {
                pass.set_vertex_buffer(1, mesh.identity.slice(..));
                pass.set_vertex_buffer(0, cap_buffer.slice(..));
                pass.draw(0..3, 0..1);
            }
        };
        // The shadow map does not move with the jitter: drawn once.
        if passes.shadow && place.first {
            look.gpu
                .encode_shadow(&self.device, &mut encoder, &self.scene_bind, mesh);
        }
        // The count of a cut-open picture, before every pass that draws
        // the cap.
        if passes.cut {
            look.gpu
                .encode_parity(&mut encoder, &self.scene_bind, mesh, &sized.parity);
        }
        let parity_bind = sized.parity_group(&look.gpu, passes.cut);
        // Layer 3: the picture without the faces that let light through,
        // shaded completely with its own geometry and occlusion, and their
        // nearest back faces (`render_studio_frame`'s `behind` and
        // `backs`).
        let through = if passes.transmit || passes.peel {
            look.through.as_ref()
        } else {
            None
        };
        let mesh_pass = |encoder: &mut wgpu::CommandEncoder,
                         label: &str,
                         colors: &[Option<wgpu::RenderPassColorAttachment<'_>>],
                         drawing: &Drawing,
                         bind: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: colors,
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.samples_depth,
                    self.two,
                    false,
                )),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, bind, &[]);
            pass.set_bind_group(2, parity_bind, &[]);
            draw_in_steps(&mut pass, &[(drawing, &draw_mesh)]);
        };
        if passes.transmit {
            let through = through.ok_or(GpuError::NothingDrawn)?;
            if passes.geometry {
                mesh_pass(
                    &mut encoder,
                    "craft3d geometry behind",
                    &[attachment(
                        &through.behind.geometry,
                        wgpu::LoadOp::Clear(studio::no_back()),
                    )],
                    &look.geometry_behind,
                    &look.gpu.uniform_bind,
                );
            }
            if passes.ao {
                look.gpu.encode_ao(
                    &self.device,
                    &mut encoder,
                    &self.scene_bind,
                    &through.behind.geometry,
                    &through.behind.ao,
                );
            }
            let bind = look.gpu.main_bind(
                &self.device,
                passes,
                mesh,
                (&through.behind.ao, &through.behind.geometry),
                None,
            )?;
            look.gpu.encode_background_into(
                &mut encoder,
                &self.scene_bind,
                &bind,
                (&through.light, &through.weight),
            );
            mesh_pass(
                &mut encoder,
                "craft3d studio scene behind",
                &[
                    attachment(&through.light, wgpu::LoadOp::Load),
                    attachment(&through.weight, wgpu::LoadOp::Load),
                ],
                &look.main_behind,
                &bind,
            );
            mesh_pass(
                &mut encoder,
                "craft3d back faces",
                &[attachment(
                    &through.backs,
                    wgpu::LoadOp::Clear(studio::no_back()),
                )],
                &look.backs,
                &look.gpu.uniform_bind,
            );
        }
        if passes.geometry {
            let empty = wgpu::Color {
                r: f64::from(f32::INFINITY.to_bits()),
                g: 0.0,
                b: 0.0,
                a: 0.0,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d geometry"),
                color_attachments: &[attachment(&sized.geometry, wgpu::LoadOp::Clear(empty))],
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.samples_depth,
                    self.two,
                    false,
                )),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, &look.gpu.uniform_bind, &[]);
            pass.set_bind_group(2, parity_bind, &[]);
            draw_in_steps(&mut pass, &[(&look.geometry, &draw_mesh)]);
        }
        if passes.ao {
            look.gpu.encode_ao(
                &self.device,
                &mut encoder,
                &self.scene_bind,
                &sized.geometry,
                &sized.ao,
            );
        }
        // A see-through picture: the surface behind the front, strictly
        // further than the front's geometry, shaded completely with its own
        // geometry and occlusion into layer 3's light behind
        // (`render_studio_frame`'s `peeled`).
        if passes.peel {
            let through = through.ok_or(GpuError::NothingDrawn)?;
            let front = look.gpu.ao_bind(&self.device, &sized.geometry);
            mesh_pass(
                &mut encoder,
                "craft3d geometry peeled",
                &[attachment(
                    &through.behind.geometry,
                    wgpu::LoadOp::Clear(studio::no_back()),
                )],
                &look.geometry_peel,
                &front,
            );
            if passes.ao {
                look.gpu.encode_ao(
                    &self.device,
                    &mut encoder,
                    &self.scene_bind,
                    &through.behind.geometry,
                    &through.behind.ao,
                );
            }
            let bind = look.gpu.main_bind_with(
                &self.device,
                passes,
                mesh,
                (&through.behind.ao, &through.behind.geometry),
                Some((&sized.geometry, &look.gpu.no_bits)),
            )?;
            look.gpu.encode_background_into(
                &mut encoder,
                &self.scene_bind,
                &bind,
                (&through.light, &through.weight),
            );
            mesh_pass(
                &mut encoder,
                "craft3d studio scene peeled",
                &[
                    attachment(&through.light, wgpu::LoadOp::Load),
                    attachment(&through.weight, wgpu::LoadOp::Load),
                ],
                &look.main_peel,
                &bind,
            );
        }
        let bind = match through {
            Some(through) if passes.peel => look.gpu.main_bind_with(
                &self.device,
                passes,
                mesh,
                (&sized.ao, &sized.geometry),
                Some((&through.light, &look.gpu.no_bits)),
            )?,
            _ => look.gpu.main_bind(
                &self.device,
                passes,
                mesh,
                (&sized.ao, &sized.geometry),
                through,
            )?,
        };
        if passes.post {
            let polish = look.polish.as_ref().ok_or(GpuError::NothingDrawn)?;
            look.gpu
                .encode_background_post(&mut encoder, &self.scene_bind, &bind, polish);
            {
                let [light_view, _] = &polish.light;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craft3d studio scene post"),
                    color_attachments: &[
                        attachment(light_view, wgpu::LoadOp::Load),
                        attachment(&polish.weight, wgpu::LoadOp::Load),
                    ],
                    depth_stencil_attachment: Some(depth_attachment(
                        &targets.samples_depth,
                        self.two,
                        false,
                    )),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_bind_group(1, &bind, &[]);
                pass.set_bind_group(2, parity_bind, &[]);
                draw_in_steps(&mut pass, &[(&look.main_post, &draw_mesh)]);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craft3d studio data"),
                    color_attachments: &[attachment(
                        &targets.samples_data,
                        wgpu::LoadOp::Clear(nothing),
                    )],
                    depth_stencil_attachment: Some(depth_attachment(
                        &targets.samples_depth,
                        self.two,
                        false,
                    )),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_bind_group(1, &look.gpu.uniform_bind, &[]);
                pass.set_bind_group(2, parity_bind, &[]);
                draw_in_steps(&mut pass, &[(&look.data, &draw_mesh)]);
            }
            look.gpu.encode_post(
                &self.device,
                &mut encoder,
                &self.scene_bind,
                passes,
                &sized.geometry,
                polish,
                &targets.samples_color,
            )?;
        } else {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craft3d background"),
                    color_attachments: &[attachment(
                        &targets.samples_color,
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    )],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&look.background);
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_bind_group(1, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d studio scene"),
                color_attachments: &[
                    attachment(&targets.samples_color, wgpu::LoadOp::Load),
                    attachment(&targets.samples_data, wgpu::LoadOp::Clear(nothing)),
                ],
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.samples_depth,
                    self.two,
                    false,
                )),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, &bind, &[]);
            draw_in_steps(&mut pass, &[(&look.main, &draw_mesh)]);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d resolve"),
                color_attachments: &[
                    attachment(&targets.color_view, wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                    attachment(&targets.data_view, wgpu::LoadOp::Clear(nothing)),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &targets.resolve_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if place.summed {
            let accumulator = look.accumulator.as_ref().ok_or(GpuError::NothingDrawn)?;
            accumulator.encode_add(
                &look.gpu,
                &self.device,
                &mut encoder,
                &targets.color_view,
                place.first,
            );
            if place.last {
                accumulator.encode_average(
                    &look.gpu,
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &targets.color_view,
                );
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Vec3;

    fn params(width: u32, height: u32, ssaa: u32) -> RenderParams {
        RenderParams {
            width,
            height,
            ssaa,
            ..RenderParams::default()
        }
    }

    #[test]
    fn sizes_are_checked_like_the_cpu_renderer_and_then_against_the_device() {
        let limit = 8192;
        assert_eq!(check_params(&params(640, 480, 2), limit), Ok(()));
        for (bad, expected) in [
            (
                params(0, 10, 1),
                RenderError::InvalidSize {
                    width: 0,
                    height: 10,
                },
            ),
            (
                params(10, MAX_SIDE + 1, 1),
                RenderError::InvalidSize {
                    width: 10,
                    height: MAX_SIDE + 1,
                },
            ),
            (params(10, 10, 0), RenderError::InvalidSsaa(0)),
            (params(10, 10, 9), RenderError::InvalidSsaa(9)),
            (
                params(MAX_SIDE, MAX_SIDE, 4),
                RenderError::TooManySamples {
                    samples: 1_u64 << 30,
                },
            ),
        ] {
            assert_eq!(check_params(&bad, limit), Err(GpuError::Render(expected)));
        }
        // Accepted by the CPU renderer, too wide for this device.
        assert_eq!(
            check_params(&params(4096, 16, 4), limit),
            Err(GpuError::TargetTooLarge {
                side: 16384,
                limit: 8192
            })
        );
        assert_eq!(check_params(&params(4096, 16, 2), limit), Ok(()));
    }

    fn axes(projection: Projection) -> ViewAxes {
        Camera {
            eye: Vec3::new(0.0, 0.0, 10.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            projection,
        }
        .axes()
        .unwrap()
    }

    #[test]
    fn the_depth_range_encloses_the_mesh_tightly() {
        let bounds = Bbox3::from_points([Vec3::new(-1.0, -1.0, -2.0), Vec3::new(1.0, 1.0, 2.0)]);
        for projection in [
            Projection::Perspective { fov_y_deg: 30.0 },
            Projection::Orthographic { half_height: 2.0 },
        ] {
            let perspective = matches!(projection, Projection::Perspective { .. });
            let (near, far) = depth_range(&axes(projection), &bounds, perspective);
            // The box spans distances 8..12 from the eye.
            assert!(near < 8.0 && near > 7.9, "near {near}");
            assert!(far > 12.0 && far < 12.1, "far {far}");
        }
        // A flat mesh seen head-on still has a range that survives f32.
        let flat = Bbox3::from_points([Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)]);
        let (near, far) = depth_range(
            &axes(Projection::Orthographic { half_height: 2.0 }),
            &flat,
            false,
        );
        assert!((near as f32) < 10.0 && (far as f32) > 10.0);
        // The eye inside the box, and the box behind the eye: valid ranges.
        let around = Bbox3::from_points([Vec3::new(-1.0, -1.0, 5.0), Vec3::new(1.0, 1.0, 20.0)]);
        let behind = Bbox3::from_points([Vec3::new(-1.0, -1.0, 15.0), Vec3::new(1.0, 1.0, 20.0)]);
        for bounds in [around, behind, Bbox3::EMPTY] {
            let (near, far) = depth_range(
                &axes(Projection::Perspective { fov_y_deg: 30.0 }),
                &bounds,
                true,
            );
            assert!(near > 0.0 && far > near, "{near} {far}");
        }
    }
}

/// The shader template in both variants: what a machine without an
/// OpenGL adapter never compiles at runtime is parsed and validated here.
#[cfg(test)]
pub(crate) mod template_tests {
    use super::scene_source_for;
    use wgpu::naga;

    /// The compositions the renderers make of `scene.wgsl`
    /// (`Renderer`/`Onscreen`, the studio look, `peel::scene_module` and `cut::module`,
    /// each for one sample per pixel or for several), with whether they
    /// are the multisampled ones.
    fn compositions() -> Vec<(&'static str, bool, Vec<&'static str>)> {
        let peel = include_str!("peel.wgsl");
        let peel_samples = include_str!("peel_samples.wgsl");
        let cut = include_str!("cut.wgsl");
        vec![
            ("scene", false, vec![]),
            (
                "scene + studio + polish",
                false,
                vec![include_str!("studio.wgsl"), include_str!("polish.wgsl")],
            ),
            ("scene + peel", false, vec![peel]),
            ("scene + peel_samples", true, vec![peel_samples]),
            (
                "scene + peel + cut + cut_flat",
                false,
                vec![peel, cut, include_str!("cut_flat.wgsl")],
            ),
            (
                "scene + peel_samples + cut + cut_samples",
                true,
                vec![peel_samples, cut, include_str!("cut_samples.wgsl")],
            ),
        ]
    }

    /// Validates `source` with naga, with `capabilities`.
    fn validate(
        label: &str,
        source: &str,
        capabilities: naga::valid::Capabilities,
    ) -> Result<naga::Module, String> {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
            .validate(&module)
            .map(|_| module)
            .map_err(|e| e.emit_to_string(source))
    }

    /// Translates every entry point of `source` to GLSL ES 3.00 as wgpu's
    /// OpenGL backend does in a browser on WebGL2 (wgpu-hal: the context's
    /// shading language version, `is_webgl`), after validating it with no
    /// optional capability. What GLSL ES 3.00 lacks and naga knows it
    /// lacks (arrays of arrays, per-sample interpolation, a sample mask,
    /// multisampled textures) fails here, on a machine without a browser.
    pub(crate) fn translate_for_webgl2(label: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        assert!(!module.entry_points.is_empty(), "{label}: no entry points");
        for entry in &module.entry_points {
            let options = naga::back::glsl::Options {
                version: naga::back::glsl::Version::Embedded {
                    version: 300,
                    is_webgl: true,
                },
                ..naga::back::glsl::Options::default()
            };
            let pipeline = naga::back::glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut out = String::new();
            naga::back::glsl::Writer::new(
                &mut out,
                &module,
                &info,
                &options,
                &pipeline,
                naga::proc::BoundsCheckPolicies::default(),
            )
            .and_then(|mut writer| writer.write().map(|_| ()))
            .unwrap_or_else(|e| panic!("{label}, {}: GLSL ES 3.00: {e}", entry.name));
            assert!(out.starts_with("#version 300 es"), "{label}: {out}");
        }
    }

    /// What OpenGL runs, single-sampled with the depth written by the
    /// fragment (`fragment_depth`), is what WebGL2 takes: every
    /// composition and the blend translate to GLSL ES 3.00. On OpenGL the
    /// pass behind the front samples the front's depth through the nearest
    /// sampler (WebGL2 reads 0 from a depth texture loaded without one,
    /// `peel::sampled_front`); everywhere else it loads it, as before.
    #[test]
    fn what_opengl_runs_translates_to_glsl_es_300() {
        for (name, multisampled, more) in compositions() {
            let peels = more.iter().any(|m| m.contains("fn fs_behind("));
            for fragment_depth in [false, true] {
                let label = format!("{name}, fragment depth {fragment_depth}");
                let source = scene_source_for(fragment_depth, multisampled, multisampled, &more);
                let (loads, samples) = reads_of(&source, "front_depth");
                if !peels {
                    assert_eq!((loads, samples), (0, 0), "{label}");
                } else if fragment_depth {
                    // OpenGL: never loaded, always sampled (the pass
                    // behind the front, and in a cut-open picture the
                    // pass behind and the cap behind).
                    assert_eq!(loads, 0, "{label}: the front's depth is loaded");
                    assert!(samples > 0, "{label}");
                } else {
                    // Every other backend: loaded, as before.
                    assert!(loads > 0, "{label}");
                    assert_eq!(samples, 0, "{label}");
                }
                if fragment_depth && !multisampled {
                    translate_for_webgl2(&format!("{name}, OpenGL"), &source);
                }
            }
        }
        translate_for_webgl2("blend", &crate::peel::blend_source(false));
        translate_for_webgl2("accumulate", include_str!("accumulate.wgsl"));
        // The sampler is declared where the layouts put it.
        assert!(
            super::FRONT_SAMPLER
                .contains(&format!("@binding({})", crate::peel::FRONT_SAMPLER_BINDING))
        );
    }

    /// How often the shader `source` loads (`textureLoad`) and samples
    /// (`textureSample*`) the texture `global`, over all its functions.
    /// WebGL2 reads 0 from a depth texture that is loaded rather than
    /// sampled with the nearest filter, and the drivers the GPU tests run
    /// on (Mesa) do not: what tells the two apart without a browser.
    pub(crate) fn reads_of(source: &str, global: &str) -> (usize, usize) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{global}: {}", e.emit_to_string(source)));
        let functions = module
            .functions
            .iter()
            .map(|(_, f)| f)
            .chain(module.entry_points.iter().map(|e| &e.function));
        let (mut loads, mut samples) = (0, 0);
        for function in functions {
            let is_global =
                |handle: naga::Handle<naga::Expression>| match function.expressions[handle] {
                    naga::Expression::GlobalVariable(g) => {
                        module.global_variables[g].name.as_deref() == Some(global)
                    }
                    _ => false,
                };
            for (_, expression) in function.expressions.iter() {
                match *expression {
                    naga::Expression::ImageLoad { image, .. } if is_global(image) => loads += 1,
                    naga::Expression::ImageSample { image, .. } if is_global(image) => {
                        samples += 1;
                    }
                    _ => {}
                }
            }
        }
        (loads, samples)
    }

    /// The multisampled cut composition on a device that cannot shade per
    /// sample (`DownlevelFlags::MULTISAMPLED_SHADING` missing: naga's
    /// capability of the same name): the per-sample count needs it, the
    /// fallback does not.
    #[test]
    fn the_cut_composition_for_a_device_without_per_sample_shading_validates() {
        let (name, multisampled, more) = compositions().pop().unwrap();
        assert!(multisampled && more.len() == 3, "{name}");
        let without =
            naga::valid::Capabilities::all() - naga::valid::Capabilities::MULTISAMPLED_SHADING;
        for fragment_depth in [false, true] {
            let fallback = scene_source_for(fragment_depth, true, false, &more);
            assert!(!fallback.contains("{{"), "{fallback}");
            assert!(!fallback.contains("@interpolate(perspective, sample)"));
            assert!(fallback.contains("@interpolate(perspective, centroid)"));
            validate("fallback", &fallback, without).unwrap();
            let per_sample = scene_source_for(fragment_depth, true, true, &more);
            assert!(
                validate("per sample", &per_sample, without).is_err(),
                "the per-sample count validated without per-sample shading"
            );
            validate("per sample", &per_sample, naga::valid::Capabilities::all()).unwrap();
        }
    }

    #[test]
    fn every_composition_validates_in_both_variants() {
        for (name, multisampled, more) in compositions() {
            for fragment_depth in [false, true] {
                let label = format!("{name}, fragment depth {fragment_depth}");
                let source = scene_source_for(fragment_depth, multisampled, multisampled, &more);
                assert!(
                    !source.contains("{{"),
                    "{label}: a marker was left: {source}"
                );
                // The count of a section is per sample where there are
                // several, and nowhere else.
                assert_eq!(
                    source.contains("@interpolate(perspective, sample)"),
                    multisampled && more.len() > 1,
                    "{label}"
                );
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
                let entries: Vec<&str> = module
                    .entry_points
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect();
                // Every fragment entry that draws with the depth test writes
                // the depth in the fragment-depth variant, and none without.
                let writes_depth = module
                    .entry_points
                    .iter()
                    .filter(|e| e.stage == naga::ShaderStage::Fragment)
                    .filter(|e| {
                        e.function
                            .result
                            .as_ref()
                            .is_some_and(|r| match &r.binding {
                                Some(naga::Binding::BuiltIn(naga::BuiltIn::FragDepth)) => true,
                                _ => match &module.types[r.ty].inner {
                                    naga::TypeInner::Struct { members, .. } => {
                                        members.iter().any(|m| {
                                            matches!(
                                                m.binding,
                                                Some(naga::Binding::BuiltIn(
                                                    naga::BuiltIn::FragDepth
                                                ))
                                            )
                                        })
                                    }
                                    _ => false,
                                },
                            })
                    })
                    .count();
                let fragments = entries.iter().filter(|e| e.starts_with("fs_")).count();
                // The count pass of a section (`fs_parity`) draws without a
                // depth attachment and never writes a depth.
                // The studio look's background and occlusion draw without
                // one too, and its shadow map tests the light's own depth,
                // which is not the view's key; so do its passes after the
                // shading (layer 2), over the screen-space buffers.
                // The studio look's count (`fs_studio_parity`) as the
                // plain one's.
                let own = [
                    "fs_parity",
                    "fs_studio_parity",
                    "fs_background",
                    "fs_ao",
                    "fs_shadow",
                    "fs_background_post",
                    "fs_reflect",
                    "fs_lens",
                    "fs_bloom_down",
                    "fs_bloom_up",
                    "fs_display",
                ];
                let with_depth = fragments - own.iter().filter(|e| entries.contains(e)).count();
                let expected = if fragment_depth { with_depth } else { 0 };
                assert_eq!(writes_depth, expected, "{label}: entries {entries:?}");
            }
        }
    }

    /// The bits of the constant expression `handle` of `arena` (literal
    /// integers, a splat or composition of them, a named constant), or
    /// `None` when it is not such a constant.
    fn constant_bits(
        module: &naga::Module,
        arena: &naga::Arena<naga::Expression>,
        handle: naga::Handle<naga::Expression>,
        depth: u32,
    ) -> Option<Vec<u32>> {
        if depth > 16 {
            return None;
        }
        match arena.try_get(handle).ok()? {
            naga::Expression::Literal(naga::Literal::U32(x)) => Some(vec![*x]),
            naga::Expression::Literal(naga::Literal::I32(x)) => Some(vec![*x as u32]),
            naga::Expression::Literal(naga::Literal::AbstractInt(x)) => Some(vec![*x as u32]),
            naga::Expression::Splat { size, value } => {
                let one = constant_bits(module, arena, *value, depth + 1)?;
                Some(one.repeat(*size as usize))
            }
            naga::Expression::Compose { components, .. } => {
                let mut bits = Vec::new();
                for component in components {
                    bits.extend(constant_bits(module, arena, *component, depth + 1)?);
                }
                Some(bits)
            }
            naga::Expression::Constant(constant) => {
                let init = module.constants.try_get(*constant).ok()?.init;
                constant_bits(module, &module.global_expressions, init, depth + 1)
            }
            _ => None,
        }
    }

    /// The places of `module` where a constant holds an infinity or a NaN:
    /// a float literal (what naga's constant evaluation folds), or a
    /// bitcast to floats of constant bits (which naga leaves as it is in
    /// a function), over the module's constant expressions and every
    /// function's and entry point's own.
    pub(crate) fn non_finite_constants(module: &naga::Module) -> Vec<String> {
        let mut found = Vec::new();
        let mut scan = |place: &str, arena: &naga::Arena<naga::Expression>| {
            for (handle, expression) in arena.iter() {
                let bad = match expression {
                    naga::Expression::Literal(naga::Literal::F32(x)) => !x.is_finite(),
                    naga::Expression::Literal(
                        naga::Literal::F64(x) | naga::Literal::AbstractFloat(x),
                    ) => !x.is_finite(),
                    naga::Expression::As {
                        expr,
                        kind: naga::ScalarKind::Float,
                        convert: None,
                    } => constant_bits(module, arena, *expr, 0)
                        .is_some_and(|bits| bits.iter().any(|b| !f32::from_bits(*b).is_finite())),
                    _ => false,
                };
                if bad {
                    found.push(format!("{place}: {expression:?} ({handle:?})"));
                }
            }
        };
        scan("module constants", &module.global_expressions);
        for (_, function) in module.functions.iter() {
            let name = function.name.clone().unwrap_or_default();
            scan(&format!("fn {name}"), &function.expressions);
        }
        for entry in &module.entry_points {
            scan(
                &format!("entry point {}", entry.name),
                &entry.function.expressions,
            );
        }
        found
    }

    /// Every WGSL module the GPU crate makes: each composition of the
    /// scene shader in each variant, the overlay's (both depth variants),
    /// the blend (both), and the shaders that stand alone. Every `.wgsl`
    /// file of the crate is in one of them (checked by name).
    fn every_module_source() -> Vec<(String, String)> {
        let mut sources = Vec::new();
        for (name, multisampled, more) in compositions() {
            for fragment_depth in [false, true] {
                sources.push((
                    format!("{name}, fragment depth {fragment_depth}"),
                    scene_source_for(fragment_depth, multisampled, multisampled, &more),
                ));
            }
        }
        for samples in [1, 4] {
            let overlay = crate::onscreen::overlay_sources(samples);
            sources.push((format!("lines, {samples}"), overlay.lines));
            sources.push((format!("fills, {samples}"), overlay.fills));
            sources.push((format!("distance, {samples}"), overlay.distance));
            sources.push((format!("depth_read, {samples}"), overlay.depth_read));
        }
        for multisampled in [false, true] {
            sources.push((
                format!("blend, {multisampled}"),
                crate::peel::blend_source(multisampled),
            ));
        }
        for (name, source) in [
            ("accumulate", include_str!("accumulate.wgsl")),
            ("edge_tiles", include_str!("edge_tiles.wgsl")),
            ("present", include_str!("present.wgsl")),
            ("resolve", include_str!("resolve.wgsl")),
        ] {
            sources.push((name.to_string(), source.to_string()));
        }
        sources
    }

    /// No module holds a constant infinity or NaN: Chrome's WGSL compiler
    /// (Tint) refuses one (`value inf cannot be represented as 'f32'`) and
    /// the whole module with it, where naga takes it. 2026-11-02: the
    /// studio shader's `bitcast<f32>` of the bits of infinity made the
    /// browser's viewport fall back to the CPU renderer, a third of a
    /// second a frame. Parsed with naga, so a named constant, a vector and
    /// a composition of templates are seen as the device sees them; the
    /// text scan of `studio.rs` is a second net.
    #[test]
    fn no_module_holds_a_constant_infinity() {
        let sources = every_module_source();
        // Every file is in a module checked here.
        let covered = [
            "scene",
            "studio",
            "polish",
            "peel",
            "peel_samples",
            "cut",
            "cut_flat",
            "cut_samples",
            "depth_test",
            "lines",
            "fills",
            "distance",
            "depth_read",
            "blend",
            "accumulate",
            "edge_tiles",
            "present",
            "resolve",
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "wgsl") {
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                assert!(
                    covered.contains(&stem.as_str()),
                    "{stem}.wgsl is in no module this test checks: add it to every_module_source"
                );
            }
        }
        for (label, source) in &sources {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
            let found = non_finite_constants(&module);
            assert!(
                found.is_empty(),
                "{label}: constant infinity or NaN: {found:?}"
            );
        }
        assert!(sources.len() >= 26, "{}", sources.len());
    }

    /// The check sees a constant infinity made in each way a shader could:
    /// a bitcast in a function, a named constant, a vector, a NaN in a
    /// named vector.
    #[test]
    fn the_infinity_check_sees_every_way_of_making_one() {
        let shaders = [
            (
                "in a function",
                "fn f() -> f32 { return bitcast<f32>(0x7f800000u); }",
            ),
            (
                "a named constant's bits",
                "const BITS: u32 = 0x7f800000u;\nfn f() -> f32 { return bitcast<f32>(BITS); }",
            ),
            (
                "a vector",
                "fn f() -> vec3<f32> { return bitcast<vec3<f32>>(vec3<u32>(0xff800000u)); }",
            ),
            (
                "a NaN",
                "const N: vec2<u32> = vec2<u32>(0x7fc00000u, 0u);\nfn f() -> vec2<f32> { return bitcast<vec2<f32>>(N); }",
            ),
        ];
        for (how, source) in shaders {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{how}: {}", e.emit_to_string(source)));
            assert!(
                !non_finite_constants(&module).is_empty(),
                "{how}: not seen in {module:#?}"
            );
        }
        // naga refuses a bitcast in a `const` declaration itself (not a
        // constant expression to it), so a constant infinity reaches a
        // module only through the ways above.
        assert!(
            naga::front::wgsl::parse_str("const NONE: f32 = bitcast<f32>(0x7f800000u);").is_err()
        );
        // And the largest finite f32 is not one.
        let module = naga::front::wgsl::parse_str(
            "const MOST: f32 = 3.4028234663852886e38;\nfn f() -> f32 { return MOST; }",
        )
        .unwrap();
        assert!(non_finite_constants(&module).is_empty());
    }
}
