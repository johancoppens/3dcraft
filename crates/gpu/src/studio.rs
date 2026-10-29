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

use crate::GpuError;
use crate::mesh::{GpuMesh, VERTEX_BYTES};
use crate::renderer::{MESH_ATTRIBUTES, uniform_entry};
use craft3d_math::Vec3;
use craft3d_render::studio::shadow::SHADOW_SIZE;
use craft3d_render::studio::{LightNumbers, Look, Prepared, ao::AO_RADIUS};
use std::sync::Arc;

/// What lights a studio picture: a prepared environment and the look.
#[derive(Clone, Debug)]
pub struct Studio {
    pub env: Arc<Prepared>,
    pub look: Look,
}

/// Number of `f32` in `studio.wgsl`'s uniform block: nine `vec4` and nine
/// for the spherical harmonics.
pub(crate) const STUDIO_FLOATS: usize = 72;
pub(crate) const GEOMETRY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const BITS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const ENV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;

/// Whether this look draws shadows, and occlusion, on this frame.
pub(crate) struct Passes {
    pub shadow: bool,
    pub ao: bool,
}

/// The uniform block for a frame of `mesh` with a target of `size`
/// samples, and which passes it needs.
pub(crate) fn studio_uniforms(
    mesh: &GpuMesh,
    studio: &Studio,
    size: (u32, u32),
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
    let bounds_radius = mesh.bounds.diagonal() * 0.5;
    let top = env.levels.len().saturating_sub(1) as f64;
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
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
    Ok((out, Passes { shadow, ao }))
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
    pub shadow: wgpu::RenderPipeline,
    pub ao: wgpu::RenderPipeline,
    pub shadow_bits: wgpu::TextureView,
    shadow_depth: wgpu::TextureView,
    /// A texel of each kind for a pass that is not drawn.
    pub no_bits: wgpu::TextureView,
    pub env: Option<EnvTextures>,
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
            entry_point: Some("vs_studio_fullscreen"),
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
            targets: &[crate::renderer::target(target)],
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
            ],
        );
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
        StudioGpu {
            uniforms,
            layout_uniform,
            layout_ao,
            layout_main,
            shadow,
            ao,
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
    ) -> Result<wgpu::BindGroup, GpuError> {
        let env = self.env.as_ref().ok_or(GpuError::NothingDrawn)?;
        let shadow = if passes.shadow {
            &self.shadow_bits
        } else {
            &self.no_bits
        };
        let ao = if passes.ao { ao } else { &self.no_bits };
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
            ],
        }))
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
