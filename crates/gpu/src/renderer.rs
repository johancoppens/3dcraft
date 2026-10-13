//! The renderer on a device someone else owns: pipelines, targets and the
//! per-frame numbers.

use crate::GpuError;
use crate::mesh::{GpuMesh, VERTEX_BYTES};
use crate::readback;
use craft3d_math::Bbox3;
use craft3d_render::{
    Camera, Frame, MAX_SAMPLES, MAX_SIDE, MAX_SSAA, NO_FACE, Projection, RenderError, RenderParams,
    SHADE_AMBIENT, SHADE_DIFFUSE, SHADE_SHININESS, SHADE_SPECULAR, ViewAxes,
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
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Number of `f32` in the scene uniform block (eight `vec4`).
const SCENE_FLOATS: usize = 32;

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
fn check_params(params: &RenderParams, max_texture_side: u32) -> Result<(), GpuError> {
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
/// with a margin: the tighter the range, the finer the depth buffer.
fn depth_range(axes: &ViewAxes, bounds: &Bbox3, perspective: bool) -> (f64, f64) {
    // Nothing to draw: any valid range will do.
    const EMPTY: (f64, f64) = (0.5, 1.0);
    let Some((lo, hi)) = bounds.corners() else {
        return EMPTY;
    };
    let (mut nearest, mut farthest) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let dist = (craft3d_math::Vec3::new(x, y, z) - axes.eye).dot(axes.forward);
                nearest = nearest.min(dist);
                farthest = farthest.max(dist);
            }
        }
    }
    if !(nearest.is_finite() && farthest.is_finite()) {
        return EMPTY;
    }
    // The margin covers the rounding of positions to f32 (relative error
    // 2^-24 ≈ 6e-8 of the mesh size) many times over, and keeps a flat mesh
    // seen head-on from having an empty range.
    let reach = nearest.abs().max(farthest.abs());
    let pad = (1e-3 * (farthest - nearest) + 1e-5 * reach).max(1e-30);
    if perspective {
        let far = farthest + pad;
        if far <= 0.0 {
            // Everything is behind the eye and is clipped anyway.
            return EMPTY;
        }
        // A camera inside the model: keep the ratio far / near bounded.
        ((nearest - pad).max(far * 1e-6), far)
    } else {
        (nearest - pad, farthest + pad)
    }
}

/// The numbers of `scene.wgsl`'s uniform block, computed in `f64` and
/// narrowed last.
fn scene_uniforms(
    mesh: &GpuMesh,
    camera: &Camera,
    params: &RenderParams,
) -> Result<[f32; SCENE_FLOATS], GpuError> {
    let axes = camera.axes()?;
    let perspective = matches!(axes.projection, Projection::Perspective { .. });
    // The same ratio as the CPU renderer's: the supersampled sizes cancel.
    let aspect = f64::from(params.width) / f64::from(params.height);
    let eye = axes.eye - mesh.origin;
    let (near, far) = depth_range(&axes, &mesh.bounds, perspective);
    let depth_scale = if perspective {
        far / (far - near)
    } else {
        1.0 / (far - near)
    };
    let light = axes.light();
    let l = light.unwrap_or(craft3d_math::Vec3::ZERO);
    let [r, g, b] = params.color.map(f64::from);
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
        [near, depth_scale, 0.0, 0.0],
        [r, g, b, 0.0],
        [
            SHADE_AMBIENT,
            SHADE_DIFFUSE,
            255.0 * SHADE_SPECULAR,
            f64::from(SHADE_SHININESS.max(0)),
        ],
    ];
    let mut out = [0.0_f32; SCENE_FLOATS];
    for (slot, value) in out.iter_mut().zip(wide.iter().flatten()) {
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

fn float_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The textures of one image size.
pub(crate) struct Targets {
    pub width: u32,
    pub height: u32,
    ssaa: u32,
    samples_color: wgpu::TextureView,
    samples_data: wgpu::TextureView,
    samples_depth: wgpu::TextureView,
    pub color: wgpu::Texture,
    pub data: wgpu::Texture,
    color_view: wgpu::TextureView,
    data_view: wgpu::TextureView,
    resolve_bind: wgpu::BindGroup,
    present_bind: wgpu::BindGroup,
}

/// A renderer on a device and queue the caller owns. It keeps its
/// pipelines and one set of targets, which it reallocates when the image
/// size or supersampling changes.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scene_pipeline: wgpu::RenderPipeline,
    scene_uniforms: wgpu::Buffer,
    scene_bind: wgpu::BindGroup,
    resolve_pipeline: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    resolve_uniforms: wgpu::Buffer,
    present_shader: wgpu::ShaderModule,
    present_layout: wgpu::BindGroupLayout,
    present_pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    targets: Option<Targets>,
    /// The buffers frames are read back through, kept between frames.
    staging: Mutex<Option<readback::Staging>>,
    drawn: bool,
    /// Identifies this renderer in the meshes it uploads.
    token: Arc<()>,
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
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

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
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

fn target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
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
        guarded(device, || Self::build(device, queue))
    }

    fn build(device: &wgpu::Device, queue: &wgpu::Queue) -> Renderer {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
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
        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d scene"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        let attributes = [
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
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d scene"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            // Both sides of every triangle are drawn: sheets are seen from
            // either side, as on the CPU.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            // `Less`: on a tie the triangle drawn first stays.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[target(COLOR_FORMAT), target(DATA_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
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
            scene_pipeline,
            scene_uniforms,
            scene_bind,
            resolve_pipeline,
            resolve_layout,
            resolve_uniforms,
            present_shader,
            present_layout,
            present_pipelines: Vec::new(),
            targets: None,
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
        guarded(&self.device, || {
            GpuMesh::upload(&self.device, &self.queue, mesh, &self.token)
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
            readback::start(&self.device, &self.queue, targets, &mut staging)
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
        let scene = scene_uniforms(mesh, camera, params)?;
        let device = self.device.clone();
        guarded(&device, || {
            self.ensure_targets(params);
            self.encode(mesh, &scene, params, present)
        })??;
        self.drawn = true;
        Ok(())
    }

    fn ensure_targets(&mut self, params: &RenderParams) {
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
        let samples_depth = view(&texture(
            "craft3d samples depth",
            DEPTH_FORMAT,
            ssaa,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
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
            color,
            data,
            color_view,
            data_view,
            resolve_bind,
            present_bind,
        });
    }

    fn encode(
        &self,
        mesh: &GpuMesh,
        scene: &[f32; SCENE_FLOATS],
        params: &RenderParams,
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d scene"),
                color_attachments: &[
                    cleared(&targets.samples_color, wgpu::Color { r, g, b, a: 1.0 }),
                    cleared(&targets.samples_data, nothing),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.samples_depth,
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
            pass.set_pipeline(&self.scene_pipeline);
            pass.set_bind_group(0, &self.scene_bind, &[]);
            // In mesh order: the depth test's tie rule depends on it.
            for chunk in &mesh.chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
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
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &targets.resolve_bind, &[]);
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
