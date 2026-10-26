//! See-through bodies (`RenderParams::opacity` below 1): what the two
//! renderers of this crate share.
//!
//! The definition is the CPU renderer's (`craft3d_render::render`,
//! *See-through*): per sample the front surface is blended over the nearest
//! surface strictly behind it. On the GPU that is depth peeling with two
//! layers: the scene is drawn twice (the front, then everything but the
//! front, of which the depth test keeps the nearest), and a last pass
//! blends the two per sample and takes the mean of the samples
//! (`blend.wgsl`). How the second pass tells the front from the rest
//! depends on the samples per pixel: with one it compares depths
//! (`peel.wgsl`), with several it compares a key of the front triangle
//! that the first pass stored per sample (`peel_samples.wgsl`). Both files
//! say why; docs/design/rendering.md has the measurements.

use crate::renderer::{
    COLOR_FORMAT, DATA_FORMAT, fragment_depth, scene_source, texture_entry, uniform_entry,
};
use std::cell::Cell;

/// Levels of a colour channel.
const LEVELS: usize = 256;

/// The blend of every pair of levels at `opacity` (within 0 to 1), with the
/// expression and the rounding of `craft3d_render::render`, in `f64`: row
/// `behind`, column `front`.
pub(crate) fn blend_table(opacity: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(LEVELS * LEVELS);
    for behind in 0..=u8::MAX {
        for front in 0..=u8::MAX {
            let mixed = f64::from(front) * opacity + f64::from(behind) * (1.0 - opacity);
            // Between two bytes: within 0..=255.
            out.push(mixed.round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// [`blend_table`] as a texture, written again when the opacity changes.
pub(crate) struct Table {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// The bits of the opacity the texture holds.
    holds: Cell<Option<u64>>,
}

impl Table {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d blend table"),
            size: wgpu::Extent3d {
                width: LEVELS as u32,
                height: LEVELS as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            holds: Cell::new(None),
        }
    }

    /// Makes the texture the table of `opacity` (within 0 to 1).
    pub fn set(&self, queue: &wgpu::Queue, opacity: f64) {
        let bits = opacity.to_bits();
        if self.holds.get() == Some(bits) {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &blend_table(opacity),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(LEVELS as u32),
                rows_per_image: Some(LEVELS as u32),
            },
            wgpu::Extent3d {
                width: LEVELS as u32,
                height: LEVELS as u32,
                depth_or_array_layers: 1,
            },
        );
        self.holds.set(Some(bits));
    }
}

/// `scene.wgsl` followed by the two passes of a see-through picture: for
/// one sample per pixel (`peel.wgsl`: `fs_front`, `fs_front_color`,
/// `fs_behind`) or for several (`peel_samples.wgsl`: `fs_front_samples`,
/// `fs_behind_samples`).
pub(crate) fn scene_module(device: &wgpu::Device, multisampled: bool) -> wgpu::ShaderModule {
    let peel = if multisampled {
        include_str!("peel_samples.wgsl")
    } else {
        include_str!("peel.wgsl")
    };
    let source = scene_source(device, &[peel]);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("craft3d scene, see-through"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// `blend.wgsl` for layers with several samples per pixel or with one.
pub(crate) fn blend_source(multisampled: bool) -> String {
    let (color, index) = if multisampled {
        ("texture_multisampled_2d<f32>", "i32(k)")
    } else {
        ("texture_2d<f32>", "0")
    };
    include_str!("blend.wgsl")
        .replace("{{COLOR}}", color)
        .replace("{{INDEX}}", index)
}

/// [`blend_source`] as a module of `device`.
pub(crate) fn blend_module(device: &wgpu::Device, multisampled: bool) -> wgpu::ShaderModule {
    let source = blend_source(multisampled);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("craft3d blend"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn layer_entry(binding: u32, multisampled: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled,
        },
        count: None,
    }
}

/// The binding, in group 1 of the pass behind the front, of the sampler
/// the front's depth is read through where [`sampled_front`] (0 is the
/// front's depth, 1 a section's count). `renderer::scene_source_for`
/// declares it there.
pub(crate) const FRONT_SAMPLER_BINDING: u32 = 2;

/// True when the pass behind the front reads the front's depth buffer
/// through a sampler that takes the nearest texel, at the texel's centre,
/// rather than loading it: on OpenGL (the backend whose fragments write
/// their depth, `renderer::fragment_depth`; it has one sample per pixel).
/// OpenGL ES 3.0, which is WebGL2 in a browser, takes a depth texture
/// whose filter is not the nearest texel for incomplete (§3.8.13) and
/// reads 0 from it, `texelFetch` too, and wgpu's OpenGL backend (wgpu-hal
/// 30.0.1) leaves a depth texture's own filter linear and binds no sampler
/// to a texture that is only loaded from. Every fragment was then behind
/// nothing and the picture showed no layer behind the front. Sampled at
/// its centre with the nearest filter the texel is read as it is, so the
/// comparison is the one of the load; the other backends keep the load.
pub(crate) fn sampled_front(device: &wgpu::Device, multisampled: bool) -> bool {
    !multisampled && fragment_depth(device)
}

/// The layout entry of that sampler.
pub(crate) fn front_sampler_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: FRONT_SAMPLER_BINDING,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
    }
}

/// A sampler that takes the nearest texel (wgpu's default filters).
pub(crate) fn front_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("craft3d front depth"),
        ..wgpu::SamplerDescriptor::default()
    })
}

/// Group 1 of the pass behind the front: the front's depth buffer (one
/// sample per pixel) or the front's keys (several), and where
/// [`sampled_front`] the sampler the depth is read through. The depth
/// buffer is bound as an unfilterable float texture, as the lines bind it
/// (naga's GLSL backend cannot `textureLoad` a depth texture).
pub(crate) fn peel_layout(device: &wgpu::Device, multisampled: bool) -> wgpu::BindGroupLayout {
    let mut entries = vec![layer_entry(0, multisampled)];
    if sampled_front(device, multisampled) {
        entries.push(front_sampler_entry());
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("craft3d front"),
        entries: &entries,
    })
}

/// The bind group of [`peel_layout`] for the front's depth buffer or keys,
/// `front`.
pub(crate) fn peel_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    multisampled: bool,
    front: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let sampler = sampled_front(device, multisampled).then(|| front_sampler(device));
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(front),
    }];
    if let Some(sampler) = &sampler {
        entries.push(wgpu::BindGroupEntry {
            binding: FRONT_SAMPLER_BINDING,
            resource: wgpu::BindingResource::Sampler(sampler),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("craft3d front"),
        layout,
        entries: &entries,
    })
}

/// The blend pass's group: the count, the two layers, the table and, for
/// the renderer that reads frames back, the front's data.
pub(crate) fn blend_layout(
    device: &wgpu::Device,
    multisampled: bool,
    data: bool,
) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
        layer_entry(1, multisampled),
        layer_entry(2, multisampled),
        layer_entry(3, false),
    ];
    if data {
        entries.push(texture_entry(4, wgpu::TextureSampleType::Uint));
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("craft3d blend"),
        entries: &entries,
    })
}

/// The blend pass: one triangle over the whole target, into the picture
/// and, with `data`, the frame's data target.
pub(crate) fn blend_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    data: bool,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("craft3d blend"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let color = Some(wgpu::ColorTargetState {
        format: COLOR_FORMAT,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    });
    let with_data = [
        color.clone(),
        Some(wgpu::ColorTargetState {
            format: DATA_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
    ];
    let only_color = [color];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("craft3d blend"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_fullscreen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(if data {
                "fs_blend_frame"
            } else {
                "fs_blend_samples"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: if data { &with_data } else { &only_color },
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The uniform block of `blend.wgsl`: `count`, then padding.
pub(crate) fn blend_uniforms(count: u32) -> Vec<u8> {
    [count, 0, 0, 0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_cpu_renderers_blend() {
        for opacity in [0.0, 0.1, 0.5, 0.9, 1.0] {
            let table = blend_table(opacity);
            assert_eq!(table.len(), LEVELS * LEVELS);
            for (front, behind) in [
                (0_u8, 0_u8),
                (255, 0),
                (0, 255),
                (5, 0),
                (198, 30),
                (7, 250),
            ] {
                let exact = f64::from(front) * opacity + f64::from(behind) * (1.0 - opacity);
                assert_eq!(
                    table[usize::from(behind) * LEVELS + usize::from(front)],
                    exact.round() as u8,
                    "{front} over {behind} at {opacity}"
                );
            }
            // A level over itself is itself.
            for level in 0..LEVELS {
                assert_eq!(usize::from(table[level * LEVELS + level]), level);
            }
        }
        // Half-way rounds away from zero, as `f64::round` does: 1 over 0.
        assert_eq!(blend_table(0.5)[1], 1);
        // Opaque: the front. Clear: the one behind.
        assert_eq!(blend_table(1.0)[200 * LEVELS + 17], 17);
        assert_eq!(blend_table(0.0)[200 * LEVELS + 17], 200);
    }
}
