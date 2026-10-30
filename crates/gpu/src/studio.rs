//! The studio look on the GPU (`studio.wgsl`; the CPU renderer's
//! `craft3d_render::studio` is the definition). The environment's levels,
//! the split-sum table and the spherical harmonics come from the CPU's
//! [`Prepared`] as they are; the GPU draws its own shadow map, geometry
//! buffer and occlusion with the same arithmetic.
//!
//! Passes, each only when the look asks for it: the shadow map (the light's
//! view, 1024², the depth from the light as `f32` bits), the geometry of
//! the samples (view distance and normal), their occlusion, the
//! background, and the surfaces, shaded in the fragment shader.
//!
//! Layer 2 (`craft3d_render::studio::polish`): frame `k` of a picture
//! moves every mesh pass's samples by `polish::jitter(k)`; the surfaces
//! take the contact shadows and the specular shadow (reading the
//! geometry); when the look has screen-space effects (`Look::post`) the
//! background and the surfaces write their linear light and specular
//! weight into integer targets, and full-screen passes (`polish.wgsl`) add
//! the reflections, the lens and bloom before the bytes; the frames of a
//! still picture are summed as integers (`accumulate.wgsl`).

use crate::GpuError;
use crate::mesh::{GpuMesh, VERTEX_BYTES};
use crate::renderer::{MESH_ATTRIBUTES, uniform_entry};
use craft3d_math::Vec3;
use craft3d_render::Camera;
use craft3d_render::studio::polish;
use craft3d_render::studio::shadow::SHADOW_SIZE;
use craft3d_render::studio::{LightNumbers, Look, Prepared, ao::AO_RADIUS};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// What lights a studio picture: a prepared environment and the look.
#[derive(Clone, Debug)]
pub struct Studio {
    pub env: Arc<Prepared>,
    pub look: Look,
}

/// Number of `f32` in `studio.wgsl`'s uniform block: fourteen `vec4`
/// (the backdrop's among them),
/// nine for the spherical harmonics and forty for the lens's taps.
pub(crate) const STUDIO_FLOATS: usize = 4 * (14 + 9 + TAP_VECTORS);
/// The `vec4` of the lens's taps, two taps in each.
const TAP_VECTORS: usize = 40;
/// The lens's taps of a frame: 8 i on ring i (`polish::dof_taps`), as
/// `polish.wgsl`'s `DOF_TAPS`.
pub(crate) const DOF_TAPS: usize = 4 * polish::DOF_RINGS * (polish::DOF_RINGS + 1);
/// The screen-space buffers: a sample's light, its weight, bloom's levels.
pub(crate) const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const GEOMETRY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const BITS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const ENV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;

/// Which passes this look draws on this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Passes {
    pub shadow: bool,
    pub ao: bool,
    /// The geometry of the samples: for the occlusion, the contact shadows,
    /// the reflections and the lens.
    pub geometry: bool,
    /// The screen-space buffers (`Look::post`): the light and the weight
    /// instead of the bytes, then the passes of `polish.wgsl`.
    pub post: bool,
    pub reflect: bool,
    pub lens: bool,
    pub bloom: bool,
    /// The sizes of bloom's levels below the picture.
    pub levels: Vec<(u32, u32)>,
    /// The frames a still picture averages (`Look::accumulate`, capped).
    pub frames: u32,
}

/// The sizes of bloom's levels below a picture of `size` samples, as
/// `polish::bloom` makes them: halved and rounded up, at most
/// `BLOOM_LEVELS`, until one texel.
pub(crate) fn bloom_levels(size: (u32, u32)) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let (mut w, mut h) = (size.0.max(1), size.1.max(1));
    for _ in 0..polish::BLOOM_LEVELS {
        if w <= 1 && h <= 1 {
            break;
        }
        (w, h) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
        out.push((w, h));
    }
    out
}

/// The share of a look's effect: the number within 0 to 1 when it is
/// above 0, else none (NaN included).
fn share(v: f64) -> f64 {
    if v > 0.0 { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// The uniform block for frame `frame` of `mesh` seen from `camera` with a
/// target of `size` samples and the picture's plain `background`, and
/// which passes it needs.
pub(crate) fn studio_uniforms(
    mesh: &GpuMesh,
    studio: &Studio,
    camera: &Camera,
    size: (u32, u32),
    frame: u32,
    background: [u8; 3],
) -> Result<([f32; STUDIO_FLOATS], Passes), GpuError> {
    let env = &studio.env;
    let look = &studio.look;
    let numbers = LightNumbers::new(&mesh.bounds, env, look);
    let origin = mesh.origin;
    let (key, key_light) = match numbers.key {
        Some((d, e, radius)) => ([d.x, d.y, d.z, 1.0], [e[0], e[1], e[2], radius]),
        None => ([0.0; 4], [0.0; 4]),
    };
    let shadow = numbers.view.is_some();
    let ao = look.ao;
    let (center, radius, u, v, w, texel) = match numbers.view {
        Some(view) => (
            view.center - origin,
            view.radius,
            view.u,
            view.v,
            view.w,
            view.texel(),
        ),
        None => (Vec3::ZERO, 1.0, Vec3::X, Vec3::Y, Vec3::Z, 1.0),
    };
    let diagonal = mesh.bounds.diagonal();
    let bounds_radius = diagonal * 0.5;
    let top = env.levels.len().saturating_sub(1) as f64;
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let (jx, jy) = polish::jitter(frame);
    let ground_reflection = share(look.ground_reflection);
    let bloom = share(look.bloom);
    let levels = if bloom > 0.0 {
        bloom_levels(size)
    } else {
        Vec::new()
    };
    let lens = look
        .dof
        .and_then(|dof| polish::Lens::new(&dof, camera, &mesh.bounds, size.1 as usize));
    let (lens_numbers, taps) = match lens {
        Some(lens) => (
            [
                lens.aperture * lens.height / (4.0 * lens.half * lens.focus),
                lens.focus,
                lens.cap,
                1.0,
            ],
            polish::dof_taps(lens.reach, frame),
        ),
        None => ([0.0; 4], Vec::new()),
    };
    if lens.is_some() && taps.len() != DOF_TAPS {
        return Err(GpuError::Unsupported("a lens with another number of taps"));
    }
    let mut wide: Vec<[f64; 4]> = vec![
        key,
        key_light,
        [center.x, center.y, center.z, radius],
        [u.x, u.y, u.z, texel],
        [v.x, v.y, v.z, flag(shadow)],
        [w.x, w.y, w.z, flag(ao)],
        [
            look.rotation_radians(),
            look.exposure_scale(),
            f64::from(look.tonemap.index()),
            top,
        ],
        [
            numbers.ground.map_or(0.0, |z| z - origin.z),
            flag(numbers.ground.is_some()),
            bounds_radius * AO_RADIUS,
            f64::from(size.1) / 4.0,
        ],
        [
            f64::from(size.0),
            f64::from(size.1),
            numbers.rest_up,
            numbers.key_luminance,
        ],
        [
            jx,
            jy,
            flag(look.contact_shadows),
            flag(look.specular_shadow),
        ],
        [
            flag(look.ssr),
            ground_reflection,
            bloom,
            (levels.len() + 1) as f64,
        ],
        [
            polish::SSR_LENGTH * diagonal,
            polish::SSR_THICKNESS * diagonal,
            polish::CONTACT_LENGTH * diagonal,
            polish::CONTACT_THICKNESS * diagonal,
        ],
        lens_numbers,
        {
            let [r, g, b] = background.map(craft3d_render::studio::tonemap::byte_to_linear);
            [r, g, b, flag(look.plain_background)]
        },
    ];
    for coef in &env.sh {
        let [r, g, b] = coef.map(f64::from);
        wide.push([r, g, b, 0.0]);
    }
    let mut out = [0.0_f32; STUDIO_FLOATS];
    for (slot, value) in out.iter_mut().zip(wide.iter().flatten()) {
        // Deliberate narrowing at the GPU boundary.
        *slot = *value as f32;
        if !slot.is_finite() {
            return Err(GpuError::OutOfRange("the studio light"));
        }
    }
    // The taps are f32 already, the numbers both renderers read.
    let first_tap = 4 * (14 + 9);
    for (slot, value) in out.iter_mut().skip(first_tap).zip(taps.iter().flatten()) {
        *slot = *value;
        if !slot.is_finite() {
            return Err(GpuError::OutOfRange("the lens"));
        }
    }
    let post = look.post();
    let passes = Passes {
        shadow,
        ao,
        geometry: ao
            || look.contact_shadows
            || look.ssr
            || look.ground_reflection > 0.0
            || look.dof.is_some(),
        post,
        reflect: post && (look.ssr || ground_reflection > 0.0),
        lens: post && lens.is_some(),
        bloom: post && bloom > 0.0,
        levels,
        frames: look.accumulate.clamp(1, polish::MAX_ACCUMULATE),
    };
    Ok((out, passes))
}

/// The environment and the table on the GPU.
pub(crate) struct EnvTextures {
    pub prepared: Arc<Prepared>,
    pub env: wgpu::TextureView,
    pub lut: wgpu::TextureView,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    mips: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: mips.max(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

impl EnvTextures {
    /// Uploads a prepared environment: its levels as the mip levels of one
    /// texture, and the split-sum table.
    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &Arc<Prepared>,
    ) -> Result<EnvTextures, GpuError> {
        let first = prepared
            .levels
            .first()
            .ok_or(GpuError::Unsupported("an environment without levels"))?;
        let size = |n: usize| u32::try_from(n).map_err(|_| GpuError::OutOfRange("the environment"));
        let (w0, h0) = (size(first.width)?, size(first.height)?);
        let mips = size(prepared.levels.len())?;
        for (i, level) in prepared.levels.iter().enumerate() {
            if (level.width, level.height) != ((w0 >> i) as usize, (h0 >> i) as usize)
                || level.texels.len() != level.width * level.height
            {
                return Err(GpuError::Unsupported(
                    "an environment whose levels do not halve",
                ));
            }
        }
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let env = texture(
            device,
            "craft3d environment",
            ENV_FORMAT,
            (w0, h0),
            mips,
            usage,
        );
        for (i, level) in prepared.levels.iter().enumerate() {
            let bytes: Vec<u8> = level
                .texels
                .iter()
                .flat_map(|t| [t[0], t[1], t[2], 1.0])
                .flat_map(f32::to_le_bytes)
                .collect();
            let (w, h) = (size(level.width)?, size(level.height)?);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &env,
                    mip_level: size(i)?,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 16),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let side = craft3d_render::studio::brdf::LUT_SIZE as u32;
        if prepared.lut.len() != (side * side) as usize {
            return Err(GpuError::Unsupported("a split-sum table of another size"));
        }
        let lut = texture(
            device,
            "craft3d split sum",
            LUT_FORMAT,
            (side, side),
            1,
            usage,
        );
        let bytes: Vec<u8> = prepared
            .lut
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 8),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        Ok(EnvTextures {
            prepared: Arc::clone(prepared),
            env: view(&env),
            lut: view(&lut),
        })
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    crate::renderer::texture_entry(binding, sample_type)
}

const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };

/// The parts of the studio look every renderer shares: the bind group
/// layouts, the uniform buffer, the passes before the surfaces and their
/// targets that do not depend on the image size.
pub(crate) struct StudioGpu {
    pub uniforms: wgpu::Buffer,
    /// Group 1 of the shadow pass (the uniform block alone), of the
    /// occlusion (with the geometry) and of the background and the
    /// surfaces (with the environment, the table, the shadow map and the
    /// occlusion).
    pub layout_uniform: wgpu::BindGroupLayout,
    pub layout_ao: wgpu::BindGroupLayout,
    pub layout_main: wgpu::BindGroupLayout,
    /// Group 1 of the passes after the shading (`polish.wgsl`).
    pub layout_post: wgpu::BindGroupLayout,
    /// Group 1 of the geometry pass (the uniform block alone).
    pub uniform_bind: wgpu::BindGroup,
    pub shadow: wgpu::RenderPipeline,
    pub ao: wgpu::RenderPipeline,
    /// The passes after the shading, at one sample per target sample.
    pub background_post: wgpu::RenderPipeline,
    reflect: wgpu::RenderPipeline,
    lens: wgpu::RenderPipeline,
    bloom_down: wgpu::RenderPipeline,
    bloom_up: wgpu::RenderPipeline,
    display: wgpu::RenderPipeline,
    /// The sum of a picture's frames and its mean (`accumulate.wgsl`).
    accumulate: wgpu::RenderPipeline,
    average: wgpu::RenderPipeline,
    accumulate_layout: wgpu::BindGroupLayout,
    accumulate_uniforms: wgpu::Buffer,
    pub shadow_bits: wgpu::TextureView,
    shadow_depth: wgpu::TextureView,
    /// A texel of each kind for a pass that is not drawn.
    pub no_bits: wgpu::TextureView,
    no_color: wgpu::TextureView,
    pub env: Option<EnvTextures>,
}

/// The shader of the studio look: the scene's with `studio.wgsl` and
/// `polish.wgsl` appended.
pub(crate) fn studio_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("craft3d studio"),
        source: wgpu::ShaderSource::Wgsl(
            crate::renderer::scene_source(
                device,
                &[include_str!("studio.wgsl"), include_str!("polish.wgsl")],
            )
            .into(),
        ),
    })
}

/// A full-screen pipeline of `studio.wgsl`.
pub(crate) fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    entry: &str,
    groups: &[Option<&wgpu::BindGroupLayout>],
    target: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    fullscreen_pipeline_to(
        device,
        label,
        module,
        ("vs_studio_fullscreen", entry),
        groups,
        &[crate::renderer::target(target)],
        samples,
    )
}

/// A full-screen pipeline with the vertex and fragment entries `entries`
/// into `targets`.
pub(crate) fn fullscreen_pipeline_to(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    (vertex, entry): (&str, &str),
    groups: &[Option<&wgpu::BindGroupLayout>],
    targets: &[Option<wgpu::ColorTargetState>],
    samples: u32,
) -> wgpu::RenderPipeline {
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
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl StudioGpu {
    /// The shared parts, with `module` the scene shader with
    /// `studio.wgsl` appended and `scene_layout` its group 0.
    pub(crate) fn new(
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> StudioGpu {
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let layout_uniform = layout("craft3d studio uniform", &[uniform_entry(0, both)]);
        let layout_ao = layout(
            "craft3d studio occlusion",
            &[
                uniform_entry(0, both),
                texture_entry(5, wgpu::TextureSampleType::Uint),
            ],
        );
        let layout_main = layout(
            "craft3d studio",
            &[
                uniform_entry(0, both),
                texture_entry(1, UNFILTERED),
                texture_entry(2, UNFILTERED),
                texture_entry(3, wgpu::TextureSampleType::Uint),
                texture_entry(4, wgpu::TextureSampleType::Uint),
                texture_entry(5, wgpu::TextureSampleType::Uint),
            ],
        );
        let layout_post = layout(
            "craft3d studio post",
            &[
                uniform_entry(0, both),
                texture_entry(1, UNFILTERED),
                texture_entry(5, wgpu::TextureSampleType::Uint),
                texture_entry(6, wgpu::TextureSampleType::Uint),
                texture_entry(7, wgpu::TextureSampleType::Uint),
                texture_entry(8, wgpu::TextureSampleType::Uint),
            ],
        );
        let accumulate_layout = layout(
            "craft3d accumulate",
            &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, UNFILTERED),
                texture_entry(2, wgpu::TextureSampleType::Uint),
            ],
        );
        let accumulate_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d accumulate"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d studio uniforms"),
            size: (STUDIO_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("craft3d shadow"),
            bind_group_layouts: &[Some(scene_layout), Some(&layout_uniform)],
            immediate_size: 0,
        });
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_ATTRIBUTES,
                })],
            },
            primitive: crate::renderer::scene_primitive(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[crate::renderer::target(BITS_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
        });
        let ao = fullscreen_pipeline(
            device,
            "craft3d occlusion",
            module,
            "fs_ao",
            &[Some(scene_layout), Some(&layout_ao)],
            BITS_FORMAT,
            1,
        );
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d studio uniform"),
            layout: &layout_uniform,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let post = |label: &str, entry: &str, format: wgpu::TextureFormat| {
            fullscreen_pipeline(
                device,
                label,
                module,
                entry,
                &[Some(scene_layout), Some(&layout_post)],
                format,
                1,
            )
        };
        let reflect = post("craft3d reflections", "fs_reflect", LIGHT_FORMAT);
        let lens = post("craft3d lens", "fs_lens", LIGHT_FORMAT);
        let bloom_down = post("craft3d bloom down", "fs_bloom_down", LIGHT_FORMAT);
        let bloom_up = post("craft3d bloom up", "fs_bloom_up", LIGHT_FORMAT);
        let display = post(
            "craft3d display",
            "fs_display",
            crate::renderer::COLOR_FORMAT,
        );
        let background_post = fullscreen_pipeline_to(
            device,
            "craft3d background post",
            module,
            ("vs_studio_fullscreen", "fs_background_post"),
            &[Some(scene_layout), Some(&layout_main)],
            &[
                crate::renderer::target(LIGHT_FORMAT),
                crate::renderer::target(LIGHT_FORMAT),
            ],
            1,
        );
        let accumulate_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d accumulate"),
            source: wgpu::ShaderSource::Wgsl(include_str!("accumulate.wgsl").into()),
        });
        let accumulate = fullscreen_pipeline_to(
            device,
            "craft3d accumulate",
            &accumulate_module,
            ("vs_accumulate", "fs_accumulate"),
            &[Some(&accumulate_layout)],
            &[crate::renderer::target(LIGHT_FORMAT)],
            1,
        );
        let average = fullscreen_pipeline_to(
            device,
            "craft3d average",
            &accumulate_module,
            ("vs_accumulate", "fs_average"),
            &[Some(&accumulate_layout)],
            &[crate::renderer::target(crate::renderer::COLOR_FORMAT)],
            1,
        );
        let side = SHADOW_SIZE as u32;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let shadow_bits = view(&texture(
            device,
            "craft3d shadow map",
            BITS_FORMAT,
            (side, side),
            1,
            attach,
        ));
        let shadow_depth = view(&texture(
            device,
            "craft3d shadow depth",
            SHADOW_DEPTH_FORMAT,
            (side, side),
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        let no_bits = view(&texture(
            device,
            "craft3d no bits",
            BITS_FORMAT,
            (1, 1),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        let no_color = view(&texture(
            device,
            "craft3d no colour",
            crate::renderer::COLOR_FORMAT,
            (1, 1),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        StudioGpu {
            no_color,
            uniforms,
            layout_uniform,
            layout_ao,
            layout_main,
            layout_post,
            uniform_bind,
            shadow,
            ao,
            background_post,
            reflect,
            lens,
            bloom_down,
            bloom_up,
            display,
            accumulate,
            average,
            accumulate_layout,
            accumulate_uniforms,
            shadow_bits,
            shadow_depth,
            no_bits,
            env: None,
        }
    }

    /// Makes `prepared` the environment on the GPU, uploading it when it
    /// is another one than the last.
    pub(crate) fn ensure_env(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &Arc<Prepared>,
    ) -> Result<(), GpuError> {
        if self
            .env
            .as_ref()
            .is_some_and(|e| Arc::ptr_eq(&e.prepared, prepared))
        {
            return Ok(());
        }
        self.env = None;
        self.env = Some(EnvTextures::upload(device, queue, prepared)?);
        Ok(())
    }

    /// The shadow pass.
    pub(crate) fn encode_shadow(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        mesh: &GpuMesh,
    ) {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d shadow"),
            layout: &self.layout_uniform,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms.as_entire_binding(),
            }],
        });
        let infinity = f64::from(f32::INFINITY.to_bits());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("craft3d shadow"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.shadow_bits,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: infinity,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_depth,
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
        pass.set_pipeline(&self.shadow);
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, &bind, &[]);
        for chunk in &mesh.chunks {
            pass.set_vertex_buffer(0, chunk.buffer.slice(..));
            pass.draw(0..chunk.vertices, 0..1);
        }
    }

    /// The occlusion pass, from `geometry` into `ao`.
    pub(crate) fn encode_ao(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        geometry: &wgpu::TextureView,
        ao: &wgpu::TextureView,
    ) {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d occlusion"),
            layout: &self.layout_ao,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(geometry),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("craft3d occlusion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ao,
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
        pass.set_pipeline(&self.ao);
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Group 1 of the background and the surfaces, with `ao` the
    /// occlusion (or [`Self::no_bits`]).
    pub(crate) fn main_bind(
        &self,
        device: &wgpu::Device,
        passes: &Passes,
        ao: &wgpu::TextureView,
        geometry: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, GpuError> {
        let env = self.env.as_ref().ok_or(GpuError::NothingDrawn)?;
        let shadow = if passes.shadow {
            &self.shadow_bits
        } else {
            &self.no_bits
        };
        let ao = if passes.ao { ao } else { &self.no_bits };
        let geometry = if passes.geometry {
            geometry
        } else {
            &self.no_bits
        };
        let tex = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d studio"),
            layout: &self.layout_main,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                tex(1, &env.env),
                tex(2, &env.lut),
                tex(3, shadow),
                tex(4, ao),
                tex(5, geometry),
            ],
        }))
    }

    /// Group 1 of a pass after the shading.
    fn post_bind(
        &self,
        device: &wgpu::Device,
        geometry: &wgpu::TextureView,
        light: &wgpu::TextureView,
        weight: &wgpu::TextureView,
        aux: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, GpuError> {
        let env = self.env.as_ref().ok_or(GpuError::NothingDrawn)?;
        let tex = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d studio post"),
            layout: &self.layout_post,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                tex(1, &env.env),
                tex(5, geometry),
                tex(6, light),
                tex(7, weight),
                tex(8, aux),
            ],
        }))
    }

    /// The background of a frame with the screen-space buffers, into
    /// `targets`' light and weight, with `bind` the bodies' group 1.
    pub(crate) fn encode_background_post(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        bind: &wgpu::BindGroup,
        targets: &PolishTargets,
    ) {
        let [first, _] = &targets.light;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("craft3d background post"),
            color_attachments: &[clear_attachment(first), clear_attachment(&targets.weight)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.background_post);
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The passes after the shading of a frame whose light and weight are
    /// in `targets` (the first light): the reflections, the lens and
    /// bloom as `passes` asks, then the bytes into `out`
    /// (`render_studio_frame`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_post(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        passes: &Passes,
        geometry: &wgpu::TextureView,
        targets: &PolishTargets,
        out: &wgpu::TextureView,
    ) -> Result<(), GpuError> {
        let geometry = if passes.geometry {
            geometry
        } else {
            &self.no_bits
        };
        let [a, b] = &targets.light;
        let (mut current, mut other) = (a, b);
        let mut run = |label: &str,
                       pipeline: &wgpu::RenderPipeline,
                       bind: &wgpu::BindGroup,
                       target: &wgpu::TextureView| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[clear_attachment(target)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, scene_bind, &[]);
            pass.set_bind_group(1, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        let none = &self.no_bits;
        if passes.reflect {
            let bind = self.post_bind(device, geometry, current, &targets.weight, none)?;
            run("craft3d reflections", &self.reflect, &bind, other);
            std::mem::swap(&mut current, &mut other);
        }
        if passes.lens {
            let bind = self.post_bind(device, geometry, current, &targets.weight, none)?;
            run("craft3d lens", &self.lens, &bind, other);
            std::mem::swap(&mut current, &mut other);
        }
        // Bloom's pyramid: down from the light, then up from the smallest
        // level, each the level plus the tent of the one below
        // (`polish::bloom`); the smallest is its own sum.
        let mut sum: Option<&wgpu::TextureView> = None;
        if passes.bloom && !targets.levels.is_empty() {
            let mut source = current;
            for level in &targets.levels {
                let bind = self.post_bind(device, geometry, none, none, source)?;
                run("craft3d bloom down", &self.bloom_down, &bind, &level.down);
                source = &level.down;
            }
            for level in targets.levels.iter().rev() {
                sum = Some(match (&level.up, sum) {
                    (Some(up), Some(coarse)) => {
                        let bind = self.post_bind(device, geometry, &level.down, none, coarse)?;
                        run("craft3d bloom up", &self.bloom_up, &bind, up);
                        up
                    }
                    _ => &level.down,
                });
            }
        }
        // The display reads the roughness: the plain background's is shown
        // without the tone curve.
        let bind = self.post_bind(
            device,
            geometry,
            current,
            &targets.weight,
            sum.unwrap_or(none),
        )?;
        run("craft3d display", &self.display, &bind, out);
        Ok(())
    }
}

/// A colour attachment cleared to zero and stored.
fn clear_attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })
}

/// One level of bloom's pyramid: the level, and the sum of it and those
/// below (none for the smallest, which is its own sum).
pub(crate) struct BloomLevel {
    down: wgpu::TextureView,
    up: Option<wgpu::TextureView>,
}

/// The screen-space buffers of a frame at its samples: two lights to
/// ping-pong between, the weight, and bloom's levels.
pub(crate) struct PolishTargets {
    pub size: (u32, u32),
    pub light: [wgpu::TextureView; 2],
    pub weight: wgpu::TextureView,
    levels: Vec<BloomLevel>,
    level_sizes: Vec<(u32, u32)>,
}

impl PolishTargets {
    pub(crate) fn new(device: &wgpu::Device, size: (u32, u32), levels: &[(u32, u32)]) -> Self {
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let make = |label: &str, size| view(&texture(device, label, LIGHT_FORMAT, size, 1, attach));
        let count = levels.len();
        PolishTargets {
            size,
            light: [make("craft3d light", size), make("craft3d light", size)],
            weight: make("craft3d weight", size),
            levels: levels
                .iter()
                .enumerate()
                .map(|(i, &s)| BloomLevel {
                    down: make("craft3d bloom level", s),
                    up: (i + 1 < count).then(|| make("craft3d bloom sum", s)),
                })
                .collect(),
            level_sizes: levels.to_vec(),
        }
    }

    /// Whether these are the targets of a frame of `size` with `levels`.
    pub(crate) fn fits(&self, size: (u32, u32), levels: &[(u32, u32)]) -> bool {
        self.size == size && self.level_sizes == levels
    }
}

/// The sum of a picture's frames, per pixel as exact integers, in two
/// targets used in turn.
pub(crate) struct Accumulator {
    pub size: (u32, u32),
    sums: [wgpu::TextureView; 2],
    /// The target with the sum so far.
    current: AtomicU32,
    /// The frames in it.
    count: AtomicU32,
}

impl Accumulator {
    pub(crate) fn new(device: &wgpu::Device, size: (u32, u32)) -> Accumulator {
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let make = || {
            view(&texture(
                device,
                "craft3d sum",
                LIGHT_FORMAT,
                size,
                1,
                attach,
            ))
        };
        Accumulator {
            size,
            sums: [make(), make()],
            current: AtomicU32::new(0),
            count: AtomicU32::new(0),
        }
    }

    fn bind(
        &self,
        gpu: &StudioGpu,
        device: &wgpu::Device,
        frame: &wgpu::TextureView,
        sum: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d accumulate"),
            layout: &gpu.accumulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: gpu.accumulate_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(frame),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(sum),
                },
            ],
        })
    }

    /// Adds the bytes of `frame` (a picture of this size) to the sum, or
    /// starts it with them when `first`.
    pub(crate) fn encode_add(
        &self,
        gpu: &StudioGpu,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        first: bool,
    ) {
        let (from, to) = match self.current.load(Ordering::Relaxed) {
            0 => (&self.sums[0], &self.sums[1]),
            _ => (&self.sums[1], &self.sums[0]),
        };
        if first {
            self.count.store(0, Ordering::Relaxed);
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d sum cleared"),
                color_attachments: &[clear_attachment(from)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let bind = self.bind(gpu, device, frame, from);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d accumulate"),
                color_attachments: &[clear_attachment(to)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.accumulate);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let next = 1 - self.current.load(Ordering::Relaxed).min(1);
        self.current.store(next, Ordering::Relaxed);
        let count = self.count.load(Ordering::Relaxed).saturating_add(1);
        self.count.store(count, Ordering::Relaxed);
    }

    /// The mean of the frames summed, rounded once, into `out` (a
    /// [`crate::renderer::COLOR_FORMAT`] target of this size).
    pub(crate) fn encode_average(
        &self,
        gpu: &StudioGpu,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        out: &wgpu::TextureView,
    ) {
        let count: Vec<u8> = [self.count.load(Ordering::Relaxed).max(1), 0, 0, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        queue.write_buffer(&gpu.accumulate_uniforms, 0, &count);
        let sum = match self.current.load(Ordering::Relaxed) {
            0 => &self.sums[0],
            _ => &self.sums[1],
        };
        let bind = self.bind(gpu, device, &gpu.no_color, sum);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("craft3d average"),
            color_attachments: &[clear_attachment(out)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&gpu.average);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The per-size targets of the passes before the surfaces: the geometry
/// with its depth, and the occlusion, at one sample per target sample.
pub(crate) struct StudioTargets {
    pub size: (u32, u32),
    pub geometry: wgpu::TextureView,
    pub ao: wgpu::TextureView,
}

impl StudioTargets {
    pub(crate) fn new(device: &wgpu::Device, size: (u32, u32)) -> StudioTargets {
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        StudioTargets {
            size,
            geometry: view(&texture(
                device,
                "craft3d geometry",
                GEOMETRY_FORMAT,
                size,
                1,
                attach,
            )),
            ao: view(&texture(
                device,
                "craft3d occlusion",
                BITS_FORMAT,
                size,
                1,
                attach,
            )),
        }
    }
}

/// The studio look of the viewport renderer (`Onscreen`): the shared
/// parts, the geometry pass at one sample per pixel with its depth, and
/// the background and the surfaces at the viewport's samples; or, with the
/// screen-space buffers or accumulation (the deferred path), everything at
/// one sample per pixel, the frames summed.
pub(crate) struct StudioOnscreen {
    pub gpu: StudioGpu,
    geometry: wgpu::RenderPipeline,
    pub background: wgpu::RenderPipeline,
    pub main: wgpu::RenderPipeline,
    /// The surfaces' light and weight at one sample per pixel.
    main_post: wgpu::RenderPipeline,
    sized: Option<(StudioTargets, wgpu::TextureView)>,
    /// The deferred path's buffers, the frame's picture and the sum.
    deferred: Option<Deferred>,
}

/// What the viewport's deferred path draws into.
struct Deferred {
    polish: PolishTargets,
    frame: wgpu::TextureView,
    accumulator: Accumulator,
}

impl StudioOnscreen {
    pub(crate) fn build(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        samples: u32,
        color_format: wgpu::TextureFormat,
    ) -> StudioOnscreen {
        let module = studio_module(device);
        let gpu = StudioGpu::new(device, &module, scene_layout);
        let mesh_pipeline = |label: &str,
                             entry: &str,
                             groups: &[Option<&wgpu::BindGroupLayout>],
                             targets: &[Option<wgpu::ColorTargetState>],
                             count: u32| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_studio"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: VERTEX_BYTES as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &MESH_ATTRIBUTES,
                    })],
                },
                primitive: crate::renderer::scene_primitive(),
                // Reversed depth, as the viewport's bodies.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::renderer::DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let target = crate::renderer::target;
        let geometry = mesh_pipeline(
            "craft3d geometry",
            "fs_geometry",
            &[Some(scene_layout), Some(&gpu.layout_uniform)],
            &[target(GEOMETRY_FORMAT)],
            1,
        );
        let main = mesh_pipeline(
            "craft3d studio bodies",
            "fs_studio_color",
            &[Some(scene_layout), Some(&gpu.layout_main)],
            &[target(color_format)],
            samples,
        );
        let main_post = mesh_pipeline(
            "craft3d studio bodies post",
            "fs_studio_post",
            &[Some(scene_layout), Some(&gpu.layout_main)],
            &[target(LIGHT_FORMAT), target(LIGHT_FORMAT)],
            1,
        );
        let background = fullscreen_pipeline(
            device,
            "craft3d background",
            &module,
            "fs_background",
            &[Some(scene_layout), Some(&gpu.layout_main)],
            color_format,
            samples,
        );
        StudioOnscreen {
            gpu,
            geometry,
            background,
            main,
            main_post,
            sized: None,
            deferred: None,
        }
    }

    /// The targets of a picture of `size` pixels; with `levels` of bloom
    /// for the deferred path when it is `Some`.
    pub(crate) fn ensure_size(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        deferred: Option<&[(u32, u32)]>,
    ) {
        if self.sized.as_ref().is_none_or(|(t, _)| t.size != size) {
            self.sized = None;
            let depth = view(&texture(
                device,
                "craft3d geometry depth",
                crate::renderer::DEPTH_FORMAT,
                size,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ));
            self.sized = Some((StudioTargets::new(device, size), depth));
        }
        let Some(levels) = deferred else {
            return;
        };
        if let Some(d) = &mut self.deferred
            && d.accumulator.size == size
        {
            if !d.polish.fits(size, levels) {
                d.polish = PolishTargets::new(device, size, levels);
            }
            return;
        }
        self.deferred = None;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        self.deferred = Some(Deferred {
            polish: PolishTargets::new(device, size, levels),
            frame: view(&texture(
                device,
                "craft3d studio frame",
                crate::renderer::COLOR_FORMAT,
                size,
                1,
                attach,
            )),
            accumulator: Accumulator::new(device, size),
        });
    }

    /// The passes before the bodies (the shadow map, the geometry and the
    /// occlusion, as `passes` asks) and, unless `deferred`, the background
    /// into `color`; then group 1 of the bodies' pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_before(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        mesh: &GpuMesh,
        passes: &Passes,
        color: &wgpu::TextureView,
        deferred: bool,
    ) -> Result<wgpu::BindGroup, GpuError> {
        let (sized, depth) = self.sized.as_ref().ok_or(GpuError::NothingDrawn)?;
        if passes.shadow {
            self.gpu.encode_shadow(device, encoder, scene_bind, mesh);
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
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sized.geometry,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(empty),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
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
            pass.set_pipeline(&self.geometry);
            pass.set_bind_group(0, scene_bind, &[]);
            pass.set_bind_group(1, &self.gpu.uniform_bind, &[]);
            for chunk in &mesh.chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
            }
        }
        if passes.ao {
            self.gpu
                .encode_ao(device, encoder, scene_bind, &sized.geometry, &sized.ao);
        }
        let bind = self
            .gpu
            .main_bind(device, passes, &sized.ao, &sized.geometry)?;
        if !deferred {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
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
            pass.set_pipeline(&self.background);
            pass.set_bind_group(0, scene_bind, &[]);
            pass.set_bind_group(1, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(bind)
    }

    /// The deferred path's frame after [`Self::encode_before`]: the
    /// background's and the bodies' light at one sample per pixel, the
    /// passes after the shading, the frame added to the sum (which it
    /// starts when `first`), and the mean into `shown`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_deferred(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene_bind: &wgpu::BindGroup,
        mesh: &GpuMesh,
        passes: &Passes,
        bind: &wgpu::BindGroup,
        first: bool,
        shown: &wgpu::TextureView,
    ) -> Result<(), GpuError> {
        let (sized, depth) = self.sized.as_ref().ok_or(GpuError::NothingDrawn)?;
        let deferred = self.deferred.as_ref().ok_or(GpuError::NothingDrawn)?;
        self.gpu
            .encode_background_post(encoder, scene_bind, bind, &deferred.polish);
        {
            let [light, _] = &deferred.polish.light;
            let load = |view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d studio bodies post"),
                color_attachments: &[load(light), load(&deferred.polish.weight)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
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
            pass.set_pipeline(&self.main_post);
            pass.set_bind_group(0, scene_bind, &[]);
            pass.set_bind_group(1, bind, &[]);
            for chunk in &mesh.chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
            }
        }
        // Without the screen-space effects (accumulation alone) this is
        // the light to bytes.
        self.gpu.encode_post(
            device,
            encoder,
            scene_bind,
            passes,
            &sized.geometry,
            &deferred.polish,
            &deferred.frame,
        )?;
        deferred
            .accumulator
            .encode_add(&self.gpu, device, encoder, &deferred.frame, first);
        deferred
            .accumulator
            .encode_average(&self.gpu, device, queue, encoder, shown);
        Ok(())
    }

    /// The deferred path's mean again into `shown`, nothing added: the
    /// picture of a frame after the last one that improves it.
    pub(crate) fn encode_average(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        shown: &wgpu::TextureView,
    ) -> Result<(), GpuError> {
        let deferred = self.deferred.as_ref().ok_or(GpuError::NothingDrawn)?;
        deferred
            .accumulator
            .encode_average(&self.gpu, device, queue, encoder, shown);
        Ok(())
    }
}

/// Whether the studio look on `device` takes the simpler path of WebGL2:
/// no shadows and no occlusion (their passes render into integer targets
/// and loop over many texels, which the browsers' WebGL2 on small GPUs
/// does slowly or not at all). The rest is the same.
pub(crate) fn simpler(device: &wgpu::Device) -> bool {
    cfg!(target_arch = "wasm32") && device.adapter_info().backend == wgpu::Backend::Gl
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// The numbers the shaders hold as constants are the CPU's.
    #[test]
    fn the_shaders_constants_are_the_cpus() {
        let studio = include_str!("studio.wgsl");
        let post = include_str!("polish.wgsl");
        let has = |source: &str, line: String| {
            assert!(source.contains(&line), "missing: {line}");
        };
        has(
            studio,
            format!("const CONTACT_STEPS: i32 = {};", polish::CONTACT_STEPS),
        );
        has(
            studio,
            format!("const CONTACT_BIAS: f32 = {:?};", polish::CONTACT_BIAS),
        );
        has(
            post,
            format!("const SSR_STEPS: i32 = {};", polish::SSR_STEPS),
        );
        has(
            post,
            format!("const SSR_REFINE: i32 = {};", polish::SSR_REFINE),
        );
        has(
            post,
            format!(
                "const SSR_MAX_ROUGHNESS: f32 = {:?};",
                polish::SSR_MAX_ROUGHNESS
            ),
        );
        has(
            post,
            format!("const SSR_EDGE: f32 = {:?};", polish::SSR_EDGE),
        );
        has(post, format!("const DOF_TAPS: i32 = {DOF_TAPS};"));
        assert_eq!(polish::dof_taps(3.0, 7).len(), DOF_TAPS);
        const { assert!(DOF_TAPS <= 2 * TAP_VECTORS) };
        // The uniform block: fourteen vectors, the harmonics, the taps.
        let block = studio
            .split("struct Studio {")
            .nth(1)
            .and_then(|s| s.split("\n}").next())
            .unwrap();
        let vectors = block.matches(": vec4<f32>,").count();
        assert_eq!(vectors, 14);
        assert!(block.contains("sh: array<vec4<f32>, 9>,"));
        assert!(block.contains(&format!("taps: array<vec4<f32>, {TAP_VECTORS}>,")));
    }

    /// Bloom's levels have the sizes of the CPU's pyramid.
    #[test]
    fn bloom_levels_halve_as_on_the_cpu() {
        for size in [(1, 1), (2, 1), (5, 3), (320, 240), (1021, 777), (64, 1)] {
            let mut cpu = Vec::new();
            let (mut w, mut h) = (size.0 as usize, size.1 as usize);
            let mut level = vec![[0.0_f32; 3]; w * h];
            for _ in 0..polish::BLOOM_LEVELS {
                if w <= 1 && h <= 1 {
                    break;
                }
                let (next, nw, nh) = polish::bloom_down(&level, w, h);
                (level, w, h) = (next, nw, nh);
                cpu.push((w as u32, h as u32));
            }
            assert_eq!(bloom_levels(size), cpu, "{size:?}");
        }
    }
}
