//! Why the viewport renderer takes one sample per pixel on wgpu's OpenGL
//! backend (`Onscreen::new`, docs/decisions.md): what each backend
//! resolves out of four samples, with the depth buffer the viewport needs
//! (multisampled, an attachment, and read by the lines' shader).
//!
//! On OpenGL (wgpu-hal 30.0.1) such a depth buffer gets no storage:
//! `glTexStorage2DMultisample(GL_TEXTURE_2D, ..)` is refused, the
//! framebuffer is incomplete, and nothing is cleared, drawn or resolved.
//! `MESA_DEBUG=1 cargo test -p craft3d-gpu --test gl_multisample --
//! --nocapture` prints Mesa's own account. The day this test fails on
//! OpenGL, wgpu has learnt it, and `Onscreen::new` can ask for four
//! samples there too.
//!
//! Needs a graphics adapter, like the other tests of this crate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

const SIZE: u32 = 8;

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("probe"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Clears a 4-sample colour target to `shade` (red and blue), resolves it
/// and returns the first pixel of the resolved texture. With
/// `depth_usage`, a 4-sample depth texture of that usage is attached too.
/// A shade of its own per call: what a failed call leaves in the resolved
/// texture is whatever was in that memory, perhaps an earlier call's.
fn resolved_clear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    depth_usage: Option<wgpu::TextureUsages>,
    two_passes: bool,
    shade: u8,
) -> [u8; 4] {
    let clear = wgpu::Color {
        r: f64::from(shade) / 255.0,
        g: 0.0,
        b: f64::from(255 - shade) / 255.0,
        a: 1.0,
    };
    let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
    let color = texture(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        4,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let shown = texture(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = depth_usage.map(|usage| {
        view(&texture(
            device,
            wgpu::TextureFormat::Depth32Float,
            4,
            usage,
        ))
    });
    let (color_view, shown_view) = (view(&color), view(&shown));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: (!two_passes).then_some(&shown_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth.as_ref().map(|view| {
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    if two_passes {
        // As the viewport does: the samples are kept by a first pass and
        // resolved by a second one that has no depth attachment.
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("probe resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: Some(&shown_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe"),
        size: u64::from(256 * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &shown,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = buffer.get_mapped_range(..).unwrap().to_vec();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

#[test]
fn four_samples_resolve_everywhere_but_with_a_depth_buffer_a_shader_reads_on_opengl() {
    let adapters = craft3d_gpu::adapters();
    assert!(!adapters.is_empty(), "no GPU adapter found");
    for adapter in adapters {
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("craft3d multisample probe"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .unwrap();
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let bound = attach | wgpu::TextureUsages::TEXTURE_BINDING;
        let gl = info.backend == wgpu::Backend::Gl;
        let mut shade = 0_u8;
        for two_passes in [false, true] {
            for (what, depth, viewport_needs_it) in [
                ("no depth buffer", None, false),
                ("a depth attachment", Some(attach), false),
                ("a depth attachment a shader reads", Some(bound), true),
            ] {
                shade += 40;
                let expected = [shade, 0, 255 - shade, 255];
                let got = resolved_clear(&device, &queue, depth, two_passes, shade);
                let label = format!(
                    "{} ({:?}), {} with {what}",
                    info.name,
                    info.backend,
                    if two_passes { "two passes" } else { "one pass" }
                );
                println!("{label}: {got:?}, cleared to {expected:?}");
                if gl && viewport_needs_it {
                    assert_ne!(
                        got, expected,
                        "{label}: wgpu's OpenGL backend resolves it now; let `Onscreen::new` \
                         ask for its samples there (and drop this assertion)"
                    );
                } else {
                    assert_eq!(got, expected, "{label}");
                }
            }
        }
    }
}
