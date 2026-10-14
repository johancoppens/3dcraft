//! The section plane (`RenderParams::section`): what the two renderers of
//! this crate share.
//!
//! The definition is the CPU renderer's (`craft3d_render::render`,
//! *Section*). On the GPU a cut-open picture is the passes of a whole one
//! with other fragment entries, which leave out what is cut away
//! (`cut.wgsl`), and two things besides:
//!
//! - **The count.** One more pass over the triangles, before the others,
//!   without a depth test, that keeps only the fragments that are cut away
//!   and writes them into an `R8Unorm` target whose blend is "one minus
//!   what was there": after it the target holds, per sample, whether the
//!   number of surfaces left out there is odd. A blend and not a stencil
//!   buffer, because the depth buffer is `Depth32Float` on every backend
//!   and has none.
//! - **The cap.** One triangle in the plane, large enough to hold the box
//!   around the mesh, drawn after the triangles of the mesh in every pass
//!   that draws them, through the same vertex stage and the same depth
//!   test (`Less`: a triangle of the mesh wins a tie, as on the CPU). Its
//!   fragments are kept where the count is odd.
//!
//! With one sample per pixel the cap reads the count at its pixel
//! (`cut_flat.wgsl`); with several it reads it per sample and keeps those
//! samples through its sample mask (`cut_samples.wgsl`). Without a
//! section none of this runs, and none of it is built before the first
//! cut-open frame.

use crate::mesh::{GpuMesh, VERTEX_BYTES};
use crate::renderer::{MESH_ATTRIBUTES, float_bytes, scene_depth, scene_primitive};
use craft3d_math::Vec3;
use craft3d_render::{Cut, RenderParams, SECTION_FACE, SECTION_MARGIN};

/// The target of the count: 0 or 1 per sample.
pub(crate) const PARITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The cap is one triangle: three vertices of the mesh's layout.
pub(crate) const CAP_BYTES: usize = 3 * VERTEX_BYTES;

/// The section of a frame, made ready for the mesh that is drawn: `None`
/// without one, and for a plane that is none.
pub(crate) fn frame_cut(mesh: &GpuMesh, params: &RenderParams) -> Option<Cut> {
    params
        .section
        .and_then(|section| Cut::new(&section, &mesh.bounds))
}

/// The four vectors of the section in `scene.wgsl`'s uniform block,
/// relative to the mesh origin.
pub(crate) fn section_uniforms(mesh: &GpuMesh, cut: Option<&Cut>) -> [[f64; 4]; 4] {
    let Some(cut) = cut else {
        // No plane, and a box nothing is in.
        return [
            [0.0; 4],
            [0.0, 0.0, 0.0, -1.0],
            [1.0, 1.0, 1.0, 0.0],
            [0.0; 4],
        ];
    };
    let n = cut.normal;
    let [r, g, b] = cut.color.map(|c| f64::from(c) / 255.0);
    let (lo, hi) = cut
        .within
        .filter(|_| cut.cap)
        .map_or((Vec3::new(1.0, 1.0, 1.0), Vec3::ZERO), |(lo, hi)| {
            (lo - mesh.origin, hi - mesh.origin)
        });
    [
        [n.x, n.y, n.z, (cut.origin - mesh.origin).dot(n)],
        [r, g, b, cut.snap],
        [lo.x, lo.y, lo.z, 0.0],
        [hi.x, hi.y, hi.z, 0.0],
    ]
}

/// The box around the mesh, grown as the box the cap is drawn in: what the
/// depth range of a cut-open frame has to hold.
pub(crate) fn cap_bounds(mesh: &GpuMesh) -> craft3d_math::Bbox3 {
    let mut bounds = mesh.bounds;
    if let Some((lo, hi)) = mesh.bounds.corners() {
        let margin = Vec3::new(1.0, 1.0, 1.0) * (mesh.bounds.diagonal() * SECTION_MARGIN);
        if margin.is_finite() {
            bounds.add(lo - margin);
            bounds.add(hi + margin);
        }
    }
    bounds
}

/// The corners of the cap: a triangle in the plane of `cut` that holds
/// the box the cap is drawn in. `None` when there is no cap.
pub(crate) fn cap_corners(cut: &Cut) -> Option<[Vec3; 3]> {
    let (lo, hi) = cut.within.filter(|_| cut.cap)?;
    let center = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5;
    // The centre of the box, moved onto the plane: every point of the box
    // that is in the plane is within `radius` of it.
    let foot = center - cut.normal * cut.side(center);
    // Two directions in the plane.
    let helper = if cut.normal.x.abs() < 0.6 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let u = cut.normal.cross(helper).normalized()?;
    let v = cut.normal.cross(u);
    // A triangle around a circle of radius 1 has its corners at distance 2;
    // a little more, so that the circle is strictly inside.
    let reach = radius * 2.05;
    let half = 3.0_f64.sqrt() * 0.5;
    Some([
        foot + v * reach,
        foot + (u * -half + v * -0.5) * reach,
        foot + (u * half + v * -0.5) * reach,
    ])
}

/// [`cap_corners`] as vertices of the mesh's layout, relative to the mesh
/// origin. `None` when there is no cap, or when the triangle does not fit
/// in `f32`.
pub(crate) fn cap_vertices(mesh: &GpuMesh, cut: &Cut) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(CAP_BYTES);
    for corner in cap_corners(cut)? {
        let p = corner - mesh.origin;
        // Deliberate narrowing at the GPU boundary.
        let position = [p.x as f32, p.y as f32, p.z as f32];
        if !position.iter().all(|c| c.is_finite()) {
            return None;
        }
        bytes.extend(float_bytes(&position));
        // No normals: the cap is not shaded.
        bytes.extend(float_bytes(&[0.0; 6]));
        bytes.extend_from_slice(&SECTION_FACE.to_le_bytes());
    }
    Some(bytes)
}

/// The buffer the cap's triangle is written into every frame.
pub(crate) fn cap_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("craft3d section cap"),
        size: CAP_BYTES as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// `scene.wgsl`, the passes of a see-through picture and the passes of a
/// cut-open one, for one sample per pixel or for several.
pub(crate) fn module(device: &wgpu::Device, multisampled: bool) -> wgpu::ShaderModule {
    let (peel, cap) = if multisampled {
        (
            include_str!("peel_samples.wgsl"),
            include_str!("cut_samples.wgsl"),
        )
    } else {
        (include_str!("peel.wgsl"), include_str!("cut_flat.wgsl"))
    };
    let source = format!(
        "{}\n{peel}\n{}\n{cap}",
        include_str!("scene.wgsl"),
        include_str!("cut.wgsl")
    );
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("craft3d scene, cut open"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn texture_entry(binding: u32, multisampled: bool) -> wgpu::BindGroupLayoutEntry {
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

/// Group 1 of a pass that draws the cap: the count at binding 1 and, for
/// the pass behind the front of a see-through picture (`front`), what
/// `peel::peel_layout` has at binding 0.
pub(crate) fn cap_layout(
    device: &wgpu::Device,
    multisampled: bool,
    front: bool,
) -> wgpu::BindGroupLayout {
    let mut entries = Vec::new();
    if front {
        entries.push(texture_entry(0, multisampled));
    }
    entries.push(texture_entry(1, multisampled));
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("craft3d section cap"),
        entries: &entries,
    })
}

/// The bind group of [`cap_layout`].
pub(crate) fn cap_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    front: Option<&wgpu::TextureView>,
    parity: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let mut entries = Vec::new();
    if let Some(front) = front {
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(front),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: 1,
        resource: wgpu::BindingResource::TextureView(parity),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("craft3d section cap"),
        layout,
        entries: &entries,
    })
}

/// The colour target of the count: every fragment turns the sample into
/// one minus what it was.
pub(crate) fn parity_target() -> Option<wgpu::ColorTargetState> {
    let invert = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDst,
        dst_factor: wgpu::BlendFactor::Zero,
        operation: wgpu::BlendOperation::Add,
    };
    let invert_alpha = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::Zero,
        operation: wgpu::BlendOperation::Add,
    };
    Some(wgpu::ColorTargetState {
        format: PARITY_FORMAT,
        blend: Some(wgpu::BlendState {
            color: invert,
            alpha: invert_alpha,
        }),
        write_mask: wgpu::ColorWrites::ALL,
    })
}

/// A pipeline over triangles of the mesh's layout with the vertex stage of
/// a cut-open picture (`vs_cut`). With `depth` it has the depth test of
/// every scene pass; the count has none.
pub(crate) fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    groups: &[Option<&wgpu::BindGroupLayout>],
    targets: &[Option<wgpu::ColorTargetState>],
    samples: u32,
    depth: bool,
) -> wgpu::RenderPipeline {
    let label = format!("craft3d cut {entry}");
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&label),
        bind_group_layouts: groups,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_cut"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_BYTES as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &MESH_ATTRIBUTES,
            })],
        },
        primitive: scene_primitive(),
        depth_stencil: depth.then(scene_depth),
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

/// A texture view of the count for a target of `width` × `height` with
/// `samples` samples per pixel.
pub(crate) fn parity_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d section count"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: PARITY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The pass that counts: every triangle of `mesh` into `parity`, cleared
/// to "even".
pub(crate) fn count_pass(
    encoder: &mut wgpu::CommandEncoder,
    parity: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    scene_bind: &wgpu::BindGroup,
    mesh: &GpuMesh,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("craft3d section count"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: parity,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, scene_bind, &[]);
    for chunk in &mesh.chunks {
        pass.set_vertex_buffer(0, chunk.buffer.slice(..));
        pass.draw(0..chunk.vertices, 0..1);
    }
}

/// Draws the cap's triangle in a pass that has drawn the mesh.
pub(crate) fn draw_cap(
    pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    buffer: &wgpu::Buffer,
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(1, bind, &[]);
    pass.set_vertex_buffer(0, buffer.slice(..));
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Bbox3;
    use craft3d_render::Section;

    fn cut(origin: Vec3, normal: Vec3, bounds: &Bbox3) -> Cut {
        Cut::new(
            &Section {
                origin,
                normal,
                cap: true,
                color: [10, 20, 30],
            },
            bounds,
        )
        .unwrap()
    }

    /// A point is inside a triangle in the plane of `n` when it is on the
    /// same side of all three edges.
    fn inside(corners: &[Vec3; 3], n: Vec3, p: Vec3) -> bool {
        let side = |a: Vec3, b: Vec3| (b - a).cross(p - a).dot(n);
        let s = [
            side(corners[0], corners[1]),
            side(corners[1], corners[2]),
            side(corners[2], corners[0]),
        ];
        s.iter().all(|&v| v > 0.0) || s.iter().all(|&v| v < 0.0)
    }

    #[test]
    fn the_caps_triangle_lies_in_the_plane_and_holds_the_box() {
        let bounds = Bbox3::from_points([Vec3::new(-3.0, 2.0, 10.0), Vec3::new(5.0, 4.0, 11.0)]);
        for (origin, normal) in [
            (Vec3::new(1.0, 3.0, 10.5), Vec3::Z),
            (Vec3::new(1.0, 3.0, 10.5), Vec3::X),
            (Vec3::new(0.0, 2.5, 10.2), Vec3::new(0.3, -0.8, 0.5)),
            (Vec3::new(4.9, 3.9, 10.9), Vec3::new(1.0, 1.0, 1.0)),
        ] {
            let cut = cut(origin, normal, &bounds);
            let corners = cap_corners(&cut).unwrap();
            for corner in corners {
                assert!(cut.side(corner).abs() < 1e-9, "a corner off the plane");
            }
            // Every point of the plane within the sphere around the box
            // the cap stays in is inside the triangle.
            let (lo, hi) = cut.within.unwrap();
            let center = (lo + hi) * 0.5;
            let radius = (hi - lo).length() * 0.5;
            let foot = center - cut.normal * cut.side(center);
            let u = (corners[2] - corners[1]).normalized().unwrap();
            let v = cut.normal.cross(u);
            for k in 0..64 {
                let angle = f64::from(k) * 0.1;
                let p = foot + (u * angle.cos() + v * angle.sin()) * radius;
                assert!(inside(&corners, cut.normal, p), "{p:?} is outside");
            }
        }
        // No cap: no triangle.
        let open = Cut {
            cap: false,
            ..cut(Vec3::ZERO, Vec3::Z, &bounds)
        };
        assert_eq!(cap_corners(&open), None);
    }

    #[test]
    fn the_count_flips_between_zero_and_one() {
        // "One minus what was there" on a target of 0 and 1.
        let target = parity_target().unwrap();
        let blend = target.blend.unwrap();
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::OneMinusDst);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::Zero);
        assert_eq!(target.format, PARITY_FORMAT);
    }

    #[test]
    fn without_a_section_the_uniforms_hold_a_box_nothing_is_in() {
        let bounds = Bbox3::from_points([Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)]);
        let ready = cut(Vec3::new(0.0, 0.0, 0.5), Vec3::Z * 3.0, &bounds);
        assert_eq!(ready.normal, Vec3::Z);
        // The margin of the cap's box is what the depth range grows by.
        let (lo, hi) = ready.within.unwrap();
        assert!(lo.x < 0.0 && hi.x > 1.0);
        assert!((hi.x - 1.0 - 3.0_f64.sqrt() * SECTION_MARGIN).abs() < 1e-12);
    }
}
