//! A mesh as the GPU wants it: three unshared vertices per triangle in
//! single precision, relative to the centre of the mesh.

use crate::GpuError;
use craft3d_math::{Bbox3, Transform, Vec3};
use craft3d_render::{FaceColors, Finish, RenderError};
use craft3d_tess::Mesh;
use std::sync::Arc;

/// Largest accepted number of triangles (2 GB of vertex data).
pub const MAX_TRIANGLES: usize = 1 << 24;

/// Position, normal, the triangle's own normal (3 × `f32` each), the face
/// id (`u32`), the face's own colour (`u32`: red, green and blue in the
/// low three bytes, and 1 in the high byte when the face has a colour of
/// its own, 0 when it is shaded from the base colour) and its finish
/// (`u32`, [`finish_word`]), and its layers of layer 3 (`u32`,
/// [`optics_word`]), and the curvature map's value at the vertex with 1
/// where there is one (2 × `f32`; 0 and 0 where there is none).
pub(crate) const VERTEX_BYTES: usize = 60;
/// The texels (`Rgba32Uint`, `f32` bits) of a row of the material table
/// ([`optics_table`]).
pub(crate) const OPTICS_TEXELS: usize = 16;
/// The bit of [`optics_word`] that marks a face that lets light through.
pub(crate) const TRANSMISSIVE_BIT: u32 = 1 << 16;
const TRIANGLE_BYTES: usize = 3 * VERTEX_BYTES;
/// Triangles per vertex buffer: 31 MB, well under every device's limit.
const CHUNK_TRIANGLES: usize = 1 << 18;

/// The vertex data of a mesh, ready to upload.
#[derive(Debug, PartialEq)]
pub(crate) struct Packed {
    /// What positions are relative to.
    pub origin: Vec3,
    /// Box around the triangles that are drawn, in model coordinates.
    pub bounds: Bbox3,
    /// Vertex bytes, at most `chunk_triangles` triangles each.
    pub chunks: Vec<Vec<u8>>,
    /// Number of triangles that are drawn.
    pub triangles: usize,
    /// The material table ([`optics_table`]), a row per distinct layers.
    pub optics: Vec<[u32; 4]>,
    /// Whether a face lets light through.
    pub transmission: bool,
}

fn narrow(v: Vec3) -> Option<[f32; 3]> {
    // Deliberate narrowing at the GPU boundary; a value that overflows
    // becomes infinite and is rejected.
    let out = [v.x as f32, v.y as f32, v.z as f32];
    out.iter().all(|c| c.is_finite()).then_some(out)
}

fn push(bytes: &mut Vec<u8>, v: [f32; 3]) {
    for c in v {
        bytes.extend_from_slice(&c.to_le_bytes());
    }
}

/// Checks `mesh` the way the CPU renderer does and lays its triangles out
/// for the vertex shader. Triangles with a corner that is not finite, or
/// not representable in `f32` relative to the mesh centre, are left out.
pub(crate) fn pack(
    mesh: &Mesh,
    colors: &FaceColors,
    values: &[f32],
    chunk_triangles: usize,
) -> Result<Packed, GpuError> {
    pack_part(mesh, 0, colors, values, chunk_triangles)
}

/// [`pack`] for a part of a scene whose face ids start at `first_face`
/// (its faces are coloured and picked by those ids). Its material table
/// is relative to its own centre; a mesh uploaded in parts makes one of
/// its own ([`GpuMesh::upload_parts`]).
fn pack_part(
    mesh: &Mesh,
    first_face: u32,
    colors: &FaceColors,
    values: &[f32],
    chunk_triangles: usize,
) -> Result<Packed, GpuError> {
    if mesh.normals.len() != mesh.positions.len() || mesh.face_ids.len() != mesh.triangles.len() {
        return Err(RenderError::InvalidMesh { triangle: 0 }.into());
    }
    if mesh.triangles.len() > MAX_TRIANGLES {
        return Err(GpuError::MeshTooLarge {
            triangles: mesh.triangles.len(),
        });
    }
    let chunk_triangles = chunk_triangles.clamp(1, CHUNK_TRIANGLES);
    // The centre of the corners of the triangles, of those that fit in
    // `f32` at all, so that one hostile coordinate cannot push the rest of
    // the mesh out of range, nor can a vertex no triangle uses (far away,
    // it would cost the drawn ones their precision).
    let representable = mesh
        .triangles
        .iter()
        .flatten()
        .filter_map(|&k| mesh.positions.get(k as usize).copied())
        .filter(|&p| narrow(p).is_some());
    let origin = Bbox3::from_points(representable)
        .center()
        .filter(|c| c.is_finite())
        .unwrap_or(Vec3::ZERO);

    let mut packed = Packed {
        origin,
        bounds: Bbox3::EMPTY,
        chunks: Vec::new(),
        triangles: 0,
        optics: optics_table(colors, origin),
        transmission: colors.has_transmission(),
    };
    let mut current: Vec<u8> = Vec::new();
    for (t, (tri, &face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
        let vertex = |k: u32| {
            let k = k as usize;
            match (mesh.positions.get(k), mesh.normals.get(k)) {
                (Some(&p), Some(&n)) => Ok((p, n)),
                _ => Err(RenderError::InvalidMesh { triangle: t }),
            }
        };
        let [ia, ib, ic] = *tri;
        let corners = [vertex(ia)?, vertex(ib)?, vertex(ic)?];
        let [(pa, _), (pb, _), (pc, _)] = corners;
        // The box of every triangle with finite corners, drawn or not:
        // the CPU renderer's `section_bounds`, so that both frame the
        // studio light alike.
        if pa.is_finite() && pb.is_finite() && pc.is_finite() {
            for p in [pa, pb, pc] {
                packed.bounds.add(p);
            }
        }
        let (Some(a), Some(b), Some(c)) = (
            narrow(pa - origin),
            narrow(pb - origin),
            narrow(pc - origin),
        ) else {
            continue;
        };
        if !(pa.is_finite() && pb.is_finite() && pc.is_finite()) {
            continue;
        }
        let flat = (pb - pa)
            .cross(pc - pa)
            .normalized()
            .and_then(narrow)
            .unwrap_or([0.0; 3]);
        if current.len() >= chunk_triangles.saturating_mul(TRIANGLE_BYTES) {
            packed.chunks.push(std::mem::take(&mut current));
        }
        if current.capacity() == 0 {
            let left = mesh.triangles.len().saturating_sub(t).min(chunk_triangles);
            current.reserve(left.saturating_mul(TRIANGLE_BYTES));
        }
        let face = face.saturating_add(first_face);
        let color = color_word(colors.get(face));
        let finish = finish_word(colors.finish(face));
        let optics = optics_word(colors, face);
        for ((position, (_, normal)), k) in [a, b, c].into_iter().zip(corners).zip([ia, ib, ic]) {
            push(&mut current, position);
            // A normal that cannot be narrowed counts as "no normal".
            push(&mut current, narrow(normal).unwrap_or([0.0; 3]));
            push(&mut current, flat);
            current.extend_from_slice(&face.to_le_bytes());
            current.extend_from_slice(&color.to_le_bytes());
            current.extend_from_slice(&finish.to_le_bytes());
            current.extend_from_slice(&optics.to_le_bytes());
            // A value that is not a number is none (the CPU's NaN).
            let value = values.get(k as usize).copied().filter(|v| v.is_finite());
            current.extend_from_slice(&value.unwrap_or(0.0).to_le_bytes());
            current.extend_from_slice(&f32::from(u8::from(value.is_some())).to_le_bytes());
        }
        packed.triangles += 1;
    }
    if !current.is_empty() {
        packed.chunks.push(current);
    }
    Ok(packed)
}

/// A face's own colour as the vertex shader takes it (see
/// [`VERTEX_BYTES`]).
pub(crate) fn color_word(color: Option<[u8; 3]>) -> u32 {
    match color {
        Some([r, g, b]) => u32::from_le_bytes([r, g, b, 1]),
        None => 0,
    }
}

/// A face's finish as the fragment shader takes it: the metallic byte in
/// the low byte and the highlight's exponent ([`Finish::shininess`]) in
/// the nine bits above it (at most [`craft3d_render::MAX_SHININESS`],
/// 256), 0 for the plain highlight, and the roughness byte plus one from
/// bit 20 (0 for none of its own), which the studio look reads. The plain
/// finish is 0.
pub(crate) fn finish_word(finish: Finish) -> u32 {
    u32::from(finish.metallic)
        | ((finish.shininess().unwrap_or(0) & 511) << 8)
        | (finish.roughness.map_or(0, |r| u32::from(r) + 1) << 20)
}

/// A face's layers as the shaders take them: its row in the material
/// table plus one in the low 16 bits (0 for none,
/// `FaceColors::optics_slot`), and [`TRANSMISSIVE_BIT`] when it lets light
/// through (the shadow map and the picture behind leave it out).
pub(crate) fn optics_word(colors: &FaceColors, face: u32) -> u32 {
    let transmissive = if colors.is_transmissive(face) {
        TRANSMISSIVE_BIT
    } else {
        0
    };
    u32::from(colors.optics_slot(face)) | transmissive
}

/// The material table of `colors` for a mesh whose positions are relative
/// to `origin`: per row of `FaceColors::optics_table`, [`OPTICS_TEXELS`]
/// texels of four `f32` (as bits), read by `studio.wgsl`'s `optics_of`:
///
/// 0. anisotropy, clear coat, its roughness, transmission;
/// 1. index of refraction, 1 when thin-walled, subsurface wrap, 1 when the
///    tangent field turns around an axis;
/// 2. the absorption coefficients, 1 when there is a subsurface colour;
/// 3. the subsurface colour's bytes;
/// 4. the field's axis;
/// 5. the origin of a field around an axis, relative to `origin` (in
///    `f32`; one that overflows is infinite, and the shader then finds no
///    tangent there);
/// 6. layer 3.5's solid texture (`studio::pattern::Pattern`), as `u32`
///    themselves: its kind (0 for none), the ramp's stops, the seed, the
///    octaves;
/// 7. its scale, distortion, grain, rays;
/// 8. its spot, the roughness at the ramp's ends, 1 when it has them;
/// 9. its origin relative to `origin`, 1 when a weave wraps the axis;
/// 10. to 12. its axes (`Pattern::frame`);
/// 13. to 15. the ramp's four colours in linear light, three numbers each.
pub(crate) fn optics_table(colors: &FaceColors, origin: Vec3) -> Vec<[u32; 4]> {
    use craft3d_render::studio::layers::TangentField;
    let mut out = Vec::with_capacity(colors.optics_table().len() * OPTICS_TEXELS);
    let flag = |b: bool| if b { 1.0_f32 } else { 0.0 };
    for o in colors.optics_table() {
        let (around, axis, from) = match o.tangent {
            TangentField::Axis(a) => (false, a, [0.0_f32; 3]),
            TangentField::Around { origin: at, axis } => {
                let rel = Vec3::new(at[0], at[1], at[2]) - origin;
                // Deliberate narrowing at the GPU boundary.
                (true, axis, [rel.x as f32, rel.y as f32, rel.z as f32])
            }
        };
        let tint = o.subsurface_color.map_or([0.0; 3], |c| c.map(f32::from));
        let rows: [[f32; 4]; 6] = [
            [
                o.anisotropy,
                o.clearcoat,
                o.clearcoat_roughness,
                o.transmission,
            ],
            [o.ior, flag(o.thin_walled), o.subsurface, flag(around)],
            [
                o.absorption[0],
                o.absorption[1],
                o.absorption[2],
                flag(o.subsurface_color.is_some()),
            ],
            [tint[0], tint[1], tint[2], 0.0],
            [axis[0], axis[1], axis[2], 0.0],
            [from[0], from[1], from[2], 0.0],
        ];
        out.extend(rows.iter().map(|r| r.map(f32::to_bits)));
        out.extend(pattern_texels(o.pattern.as_ref(), origin));
    }
    out
}

/// Texels 6 to 15 of a row of the material table ([`optics_table`]): the
/// solid texture, or zeros for none.
fn pattern_texels(
    pattern: Option<&craft3d_render::studio::pattern::Pattern>,
    origin: Vec3,
) -> [[u32; 4]; OPTICS_TEXELS - 6] {
    use craft3d_render::studio::pattern::stops_linear;
    let mut out = [[0_u32; 4]; OPTICS_TEXELS - 6];
    let Some(p) = pattern else {
        return out;
    };
    let flag = |b: bool| if b { 1.0_f32 } else { 0.0 };
    let rel = Vec3::new(p.origin[0], p.origin[1], p.origin[2]) - origin;
    let [fx, fy, fz] = p.frame();
    let [r0, r1] = p.roughness.unwrap_or([0.0; 2]);
    let ramp: Vec<f32> = p.ramp.iter().flat_map(|c| c.map(stops_linear)).collect();
    let at = |k: usize| ramp.get(k).copied().unwrap_or(0.0);
    let floats: [[f32; 4]; OPTICS_TEXELS - 7] = [
        [p.scale, p.distortion, p.grain, p.rays],
        [p.spot, r0, r1, flag(p.roughness.is_some())],
        // Deliberate narrowing at the GPU boundary.
        [rel.x as f32, rel.y as f32, rel.z as f32, flag(p.wrap)],
        [fx[0], fx[1], fx[2], 0.0],
        [fy[0], fy[1], fy[2], 0.0],
        [fz[0], fz[1], fz[2], 0.0],
        [at(0), at(1), at(2), at(3)],
        [at(4), at(5), at(6), at(7)],
        [at(8), at(9), at(10), at(11)],
    ];
    if let Some(first) = out.first_mut() {
        *first = [p.kind.code(), u32::from(p.stops), p.seed, p.detail];
    }
    for (o, f) in out.iter_mut().skip(1).zip(floats) {
        *o = f.map(f32::to_bits);
    }
    out
}

pub(crate) struct Chunk {
    pub buffer: wgpu::Buffer,
    pub vertices: u32,
    /// The part of the mesh it belongs to: which pose it is drawn with.
    pub part: u32,
}

/// One part of a mesh uploaded in parts ([`crate::Renderer::upload_parts`]):
/// a body's mesh in its own coordinates, where its face ids start in the
/// scene, and the rigid motion it is drawn with.
#[derive(Clone, Copy, Debug)]
pub struct MeshPart<'a> {
    pub mesh: &'a Mesh,
    pub first_face_id: u32,
    pub pose: Transform,
}

/// Bytes of one pose on the GPU: three rows of four `f32` (the rotation's
/// row and the translation's component), a vertex buffer stepped per
/// instance ([`POSE_ATTRIBUTES`]).
pub(crate) const POSE_BYTES: usize = 48;

/// The pose's rows at the shader locations after the mesh's.
pub(crate) const POSE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 8,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 9,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 10,
    },
];

/// The two vertex buffers of every pipeline that draws a mesh: the
/// vertices ([`VERTEX_BYTES`], `attributes`) and the pose of the part
/// ([`POSE_BYTES`]), one per draw.
pub(crate) fn mesh_buffers(
    attributes: &[wgpu::VertexAttribute],
) -> [Option<wgpu::VertexBufferLayout<'_>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: POSE_BYTES as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &POSE_ATTRIBUTES,
        }),
    ]
}

/// The pose bytes of the rigid motion `pose` for a part whose vertices
/// are relative to `part_origin`, drawn relative to `mesh_origin`:
/// `p ↦ R p + (R c + t − O)`, worked out in `f64` and narrowed once.
/// `None` when it does not fit in `f32`.
pub(crate) fn pose_bytes(
    pose: &Transform,
    part_origin: Vec3,
    mesh_origin: Vec3,
) -> Option<[u8; POSE_BYTES]> {
    let shift = pose.point(part_origin) - mesh_origin;
    let rows = [
        pose.vector(Vec3::X),
        pose.vector(Vec3::Y),
        pose.vector(Vec3::Z),
    ];
    // `rows` are the columns of R; row r of R is (X_r, Y_r, Z_r).
    let mut out = [0_u8; POSE_BYTES];
    let row = |r: usize| -> Option<[f32; 4]> {
        let pick = |v: Vec3| match r {
            0 => v.x,
            1 => v.y,
            _ => v.z,
        };
        let values = [
            pick(rows[0]) as f32,
            pick(rows[1]) as f32,
            pick(rows[2]) as f32,
            pick(shift) as f32,
        ];
        values.iter().all(|c| c.is_finite()).then_some(values)
    };
    for (r, slot) in out.as_chunks_mut::<16>().0.iter_mut().enumerate() {
        let values = row(r)?;
        for (c, bytes) in values.iter().zip(slot.as_chunks_mut::<4>().0.iter_mut()) {
            *bytes = c.to_le_bytes();
        }
    }
    Some(out)
}

/// A mesh on the GPU, made by [`crate::Renderer::upload`]. It belongs to the
/// renderer that made it: another renderer refuses to draw it.
pub struct GpuMesh {
    pub(crate) chunks: Vec<Chunk>,
    /// One pose per part ([`POSE_BYTES`] each): the identity for a mesh
    /// uploaded whole.
    pub(crate) poses: wgpu::Buffer,
    /// What each part's vertices are relative to, in its own
    /// coordinates; one entry for a mesh uploaded whole.
    pub(crate) part_origins: Vec<Vec3>,
    /// One identity pose: what a triangle given relative to
    /// [`Self::origin`] (the section's cap) is drawn with.
    pub(crate) identity: wgpu::Buffer,

    pub(crate) origin: Vec3,
    pub(crate) bounds: Bbox3,
    triangles: usize,
    /// The token of the renderer that made this mesh.
    pub(crate) owner: Arc<()>,
    /// Which upload this is: the viewport's accumulation restarts for
    /// another mesh.
    pub(crate) serial: u64,
    /// The material table of layer 3 ([`optics_table`]), when a face has
    /// layers.
    pub(crate) optics: Option<wgpu::TextureView>,
    /// Whether a face lets light through (layer 3's transmission).
    pub(crate) transmission: bool,
}

/// The serial of the next upload.
static NEXT_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl GpuMesh {
    /// Number of triangles that are drawn (those of the mesh, less the ones
    /// with a corner the GPU cannot represent).
    pub fn triangle_count(&self) -> usize {
        self.triangles
    }

    /// The point the positions on the GPU are relative to: the centre of
    /// the mesh.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The box around the triangles that are drawn.
    pub fn bounds(&self) -> Bbox3 {
        self.bounds
    }

    /// How many parts it was uploaded in (1 for a mesh uploaded whole).
    pub fn part_count(&self) -> usize {
        self.part_origins.len()
    }

    /// Draws every chunk with its part's pose.
    pub(crate) fn draw_in(&self, pass: &mut wgpu::RenderPass<'_>) {
        for chunk in &self.chunks {
            let at = u64::from(chunk.part).saturating_mul(POSE_BYTES as u64);
            pass.set_vertex_buffer(
                1,
                self.poses.slice(at..at.saturating_add(POSE_BYTES as u64)),
            );
            pass.set_vertex_buffer(0, chunk.buffer.slice(..));
            pass.draw(0..chunk.vertices, 0..1);
        }
    }

    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &Mesh,
        colors: &FaceColors,
        values: &[f32],
        owner: &Arc<()>,
    ) -> Result<GpuMesh, GpuError> {
        let per_chunk = per_chunk(device)?;
        let packed = pack(mesh, colors, values, per_chunk)?;
        let mut chunks = Vec::with_capacity(packed.chunks.len());
        push_chunks(device, queue, &packed, 0, mesh, &mut chunks)?;
        let identity =
            pose_bytes(&Transform::IDENTITY, Vec3::ZERO, Vec3::ZERO).ok_or_else(|| {
                GpuError::Device("the identity does not fit in single precision".to_string())
            })?;
        let optics = optics_view(device, queue, &packed.optics)?;
        Ok(GpuMesh {
            optics,
            transmission: packed.transmission,
            chunks,
            poses: pose_buffer(device, queue, &identity),
            identity: pose_buffer(device, queue, &identity),
            part_origins: vec![Vec3::ZERO],
            origin: packed.origin,
            bounds: packed.bounds,
            triangles: packed.triangles,
            owner: Arc::clone(owner),
            serial: NEXT_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        })
    }
    /// Uploads `parts` (a scene's bodies, each in its own coordinates)
    /// once, each drawn with its pose; `bounds` is the box of the
    /// triangles as placed ([`drawn_bounds`] of the placed scene's mesh),
    /// which the depth range, the section and the studio light are
    /// framed on. Its origin is the centre of `bounds`.
    pub(crate) fn upload_parts(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        parts: &[MeshPart<'_>],
        colors: &FaceColors,
        bounds: Bbox3,
        owner: &Arc<()>,
    ) -> Result<GpuMesh, GpuError> {
        let total = parts
            .iter()
            .fold(0usize, |n, p| n.saturating_add(p.mesh.triangles.len()));
        if total > MAX_TRIANGLES {
            return Err(GpuError::MeshTooLarge { triangles: total });
        }
        let per_chunk = per_chunk(device)?;
        let origin = bounds
            .center()
            .filter(|c| c.is_finite())
            .unwrap_or(Vec3::ZERO);
        let mut chunks = Vec::new();
        let mut part_origins = Vec::with_capacity(parts.len());
        let mut triangles = 0usize;
        for (k, part) in parts.iter().enumerate() {
            let packed = pack_part(part.mesh, part.first_face_id, colors, &[], per_chunk)?;
            let index =
                u32::try_from(k).map_err(|_| GpuError::MeshTooLarge { triangles: total })?;
            push_chunks(device, queue, &packed, index, part.mesh, &mut chunks)?;
            part_origins.push(packed.origin);
            triangles = triangles.saturating_add(packed.triangles);
        }
        let identity =
            pose_bytes(&Transform::IDENTITY, Vec3::ZERO, Vec3::ZERO).ok_or_else(|| {
                GpuError::Device("the identity does not fit in single precision".to_string())
            })?;
        // One material table for all the parts, relative to the mesh's
        // origin as the moved vertices are.
        let optics = optics_view(device, queue, &optics_table(colors, origin))?;
        let mut mesh = GpuMesh {
            optics,
            transmission: colors.has_transmission(),
            chunks,
            poses: pose_buffer(device, queue, &identity),
            identity: pose_buffer(device, queue, &identity),
            part_origins,
            origin,
            bounds,
            triangles,
            owner: Arc::clone(owner),
            serial: 0,
        };
        let poses: Vec<Transform> = parts.iter().map(|p| p.pose).collect();
        mesh.set_poses(device, queue, &poses, bounds)?;
        Ok(mesh)
    }

    /// Moves the parts to `poses` (one per part, in the order uploaded):
    /// writes a few matrices, not the geometry. `bounds` is the box of
    /// the triangles as now placed. The origin stays where it was.
    pub(crate) fn set_poses(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        poses: &[Transform],
        bounds: Bbox3,
    ) -> Result<(), GpuError> {
        if poses.len() != self.part_origins.len() {
            return Err(GpuError::Device(format!(
                "{} poses for a mesh of {} parts",
                poses.len(),
                self.part_origins.len()
            )));
        }
        let mut bytes = Vec::with_capacity(poses.len().saturating_mul(POSE_BYTES));
        for (pose, origin) in poses.iter().zip(&self.part_origins) {
            let one = pose_bytes(pose, *origin, self.origin).ok_or_else(|| {
                GpuError::Device("a pose does not fit in single precision".to_string())
            })?;
            bytes.extend_from_slice(&one);
        }
        if (self.poses.size() as usize) == bytes.len() {
            queue.write_buffer(&self.poses, 0, &bytes);
        } else {
            self.poses = pose_buffer(device, queue, &bytes);
        }
        self.bounds = bounds;
        self.serial = NEXT_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

/// The material table `optics` ([`optics_table`]) on the GPU, `None` when
/// no face has layers.
fn optics_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    optics: &[[u32; 4]],
) -> Result<Option<wgpu::TextureView>, GpuError> {
    let optics = if optics.is_empty() {
        None
    } else {
        let rows = optics.len() / OPTICS_TEXELS;
        let height = u32::try_from(rows)
            .ok()
            .filter(|&h| h <= device.limits().max_texture_dimension_2d)
            .ok_or(GpuError::Unsupported("more layers than the device holds"))?;
        let size = wgpu::Extent3d {
            width: OPTICS_TEXELS as u32,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d optics"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = optics
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((OPTICS_TEXELS * 16) as u32),
                rows_per_image: Some(height),
            },
            size,
        );
        Some(texture.create_view(&wgpu::TextureViewDescriptor::default()))
    };
    Ok(optics)
}

/// Triangles per vertex buffer on `device`.
fn per_chunk(device: &wgpu::Device) -> Result<usize, GpuError> {
    let by_limit = device.limits().max_buffer_size / TRIANGLE_BYTES as u64;
    let per_chunk = usize::try_from(by_limit).unwrap_or(usize::MAX);
    if per_chunk == 0 {
        return Err(GpuError::Device(
            "the device's buffers are too small for one triangle".to_string(),
        ));
    }
    Ok(per_chunk)
}

/// Puts `packed`'s chunks on the GPU as chunks of part `part`.
fn push_chunks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    packed: &Packed,
    part: u32,
    mesh: &Mesh,
    chunks: &mut Vec<Chunk>,
) -> Result<(), GpuError> {
    for bytes in &packed.chunks {
        let vertices =
            u32::try_from(bytes.len() / VERTEX_BYTES).map_err(|_| GpuError::MeshTooLarge {
                triangles: mesh.triangles.len(),
            })?;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d mesh"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        chunks.push(Chunk {
            buffer,
            vertices,
            part,
        });
    }
    Ok(())
}

/// A vertex buffer holding `bytes` of poses.
fn pose_buffer(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("craft3d poses"),
        size: bytes.len().max(POSE_BYTES) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    buffer
}

/// The box [`GpuMesh::bounds`] is for `mesh` uploaded whole: around
/// every triangle with finite corners, as the CPU renderer frames its
/// section and studio light. For [`crate::Renderer::upload_parts`] and
/// [`crate::Renderer::set_poses`], of the scene's mesh as placed.
pub fn drawn_bounds(mesh: &Mesh) -> Bbox3 {
    let mut bounds = Bbox3::EMPTY;
    for tri in &mesh.triangles {
        let corners = tri.map(|k| mesh.positions.get(k as usize).copied());
        if let [Some(a), Some(b), Some(c)] = corners
            && a.is_finite()
            && b.is_finite()
            && c.is_finite()
        {
            for p in [a, b, c] {
                bounds.add(p);
            }
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle(mesh: &mut Mesh, corners: [Vec3; 3], face: u32) {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(corners);
        mesh.normals.extend([Vec3::Z; 3]);
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.face_ids.push(face);
    }

    fn floats(bytes: &[u8]) -> Vec<f32> {
        let (words, _) = bytes.as_chunks::<4>();
        words.iter().map(|b| f32::from_le_bytes(*b)).collect()
    }

    #[test]
    fn vertices_are_relative_to_the_mesh_centre_and_carry_the_face() {
        let mut mesh = Mesh::default();
        // Far from the origin: the offsets, not the coordinates, are stored.
        let far = Vec3::new(1e9, -2e9, 3e9);
        triangle(
            &mut mesh,
            [
                far,
                far + Vec3::new(2.0, 0.0, 0.0),
                far + Vec3::new(0.0, 4.0, 0.0),
            ],
            0xDEAD_BEEF,
        );
        let mut colors = FaceColors::new();
        assert!(colors.set(7, Some([1, 2, 3])));
        let packed = pack(&mesh, &colors, &[], 1000).unwrap();
        assert_eq!(packed.triangles, 1);
        assert_eq!(packed.origin, far + Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(packed.chunks.len(), 1);
        let bytes = &packed.chunks[0];
        assert_eq!(bytes.len(), 3 * VERTEX_BYTES);
        let first = floats(&bytes[..36]);
        assert_eq!(first[..3], [-1.0, -2.0, 0.0]); // position
        assert_eq!(first[3..6], [0.0, 0.0, 1.0]); // normal
        assert_eq!(first[6..9], [0.0, 0.0, 1.0]); // the triangle's own normal
        assert_eq!(bytes[36..40], 0xDEAD_BEEF_u32.to_le_bytes());
        // That face has no colour of its own, nor a finish.
        assert_eq!(bytes[40..44], [0, 0, 0, 0]);
        assert_eq!(bytes[44..48], [0, 0, 0, 0]);
        // No value for the curvature map: 0, and 0 for "none".
        assert_eq!(floats(&bytes[52..60]), [0.0, 0.0]);
        let second = floats(&bytes[VERTEX_BYTES..VERTEX_BYTES + 12]);
        assert_eq!(second, [1.0, -2.0, 0.0]);
        let (lo, hi) = packed.bounds.corners().unwrap();
        assert_eq!((lo, hi), (far, far + Vec3::new(2.0, 4.0, 0.0)));
        // A face with a colour of its own carries it, flagged, at every
        // corner.
        let mut seven = Mesh::default();
        triangle(&mut seven, [Vec3::ZERO, Vec3::X, Vec3::Y], 7);
        let bytes = &pack(&seven, &colors, &[], 1000).unwrap().chunks[0];
        for corner in 0..3 {
            let at = corner * VERTEX_BYTES + 40;
            assert_eq!(bytes[at..at + 4], [1, 2, 3, 1]);
            assert_eq!(bytes[at + 4..at + 8], [0, 0, 0, 0]);
        }
        // With a finish: the metallic byte, the exponent above it and the
        // roughness byte plus one from bit 20.
        let finish = Finish {
            metallic: 200,
            roughness: Some(255),
        };
        assert_eq!(finish_word(finish), 200 | (1 << 8) | (256 << 20));
        assert!(colors.set_look(7, Some(([1, 2, 3], finish))));
        let bytes = &pack(&seven, &colors, &[], 1000).unwrap().chunks[0];
        for corner in 0..3 {
            let at = corner * VERTEX_BYTES + 44;
            assert_eq!(bytes[at..at + 4], [200, 1, 0, 0x10]);
        }
        assert_eq!(
            finish_word(Finish {
                metallic: 0,
                roughness: Some(0)
            }),
            (256 << 8) | (1 << 20)
        );
        let (lo, hi) = packed.bounds.corners().unwrap();
        assert_eq!((lo, hi), (far, far + Vec3::new(2.0, 4.0, 0.0)));
    }

    /// Layer 3: each corner carries its face's slot and transmissive bit,
    /// and the table holds the layers relative to the mesh origin.
    #[test]
    fn layers_go_into_the_vertices_and_the_table() {
        use craft3d_render::studio::layers::{Optics, TangentField};
        let mut mesh = Mesh::default();
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 3);
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X * 4.0, Vec3::Y * 2.0], 5);
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 6);
        let mut colors = FaceColors::new();
        let brushed = Optics {
            anisotropy: 0.5,
            tangent: TangentField::Around {
                origin: [10.0, 20.0, 30.0],
                axis: [0.0, 0.0, 1.0],
            },
            ..Optics::default()
        };
        let glass = Optics {
            transmission: 1.0,
            ior: 1.4,
            thin_walled: true,
            absorption: [0.5, 0.25, 0.125],
            subsurface: 0.25,
            subsurface_color: Some([1, 2, 3]),
            ..Optics::default()
        };
        assert!(colors.set_optics(3, Some(brushed)));
        assert!(colors.set_optics(5, Some(glass)));
        let packed = pack(&mesh, &colors, &[], 1000).unwrap();
        assert!(packed.transmission);
        let word = |t: usize, corner: usize| {
            let at = (3 * t + corner) * VERTEX_BYTES + 48;
            u32::from_le_bytes(packed.chunks[0][at..at + 4].try_into().unwrap())
        };
        for corner in 0..3 {
            assert_eq!(word(0, corner), 1);
            assert_eq!(word(1, corner), 2 | TRANSMISSIVE_BIT);
            assert_eq!(word(2, corner), 0);
        }
        assert_eq!(packed.optics.len(), 2 * OPTICS_TEXELS);
        let row = |r: usize, k: usize| packed.optics[r * OPTICS_TEXELS + k].map(f32::from_bits);
        assert_eq!(row(0, 0), [0.5, 0.0, 0.0, 0.0]);
        assert_eq!(row(0, 1), [1.5, 0.0, 0.0, 1.0]);
        assert_eq!(row(0, 4), [0.0, 0.0, 1.0, 0.0]);
        let o = packed.origin;
        assert_eq!(
            row(0, 5),
            [
                (10.0 - o.x) as f32,
                (20.0 - o.y) as f32,
                (30.0 - o.z) as f32,
                0.0
            ]
        );
        assert_eq!(row(1, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(row(1, 1), [1.4, 1.0, 0.25, 0.0]);
        assert_eq!(row(1, 2), [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(row(1, 3), [1.0, 2.0, 3.0, 0.0]);
        // Without layers: no table, no bit, every word 0.
        let plain = pack(&mesh, &FaceColors::new(), &[], 1000).unwrap();
        assert!(plain.optics.is_empty() && !plain.transmission);
    }

    #[test]
    fn triangles_are_split_over_buffers_in_order() {
        let mut mesh = Mesh::default();
        for i in 0..7 {
            let x = f64::from(i);
            triangle(
                &mut mesh,
                [
                    Vec3::new(x, 0.0, 0.0),
                    Vec3::new(x + 1.0, 0.0, 0.0),
                    Vec3::new(x, 1.0, 0.0),
                ],
                i,
            );
        }
        let packed = pack(&mesh, &FaceColors::new(), &[], 3).unwrap();
        let sizes: Vec<usize> = packed
            .chunks
            .iter()
            .map(|c| c.len() / TRIANGLE_BYTES)
            .collect();
        assert_eq!(sizes, [3, 3, 1]);
        let faces: Vec<u32> = packed
            .chunks
            .iter()
            .flat_map(|c| c.as_chunks::<TRIANGLE_BYTES>().0)
            .map(|t| u32::from_le_bytes([t[36], t[37], t[38], t[39]]))
            .collect();
        assert_eq!(faces, [0, 1, 2, 3, 4, 5, 6]);
        // The same bytes whatever the chunk size.
        assert_eq!(
            packed.chunks.concat(),
            pack(&mesh, &FaceColors::new(), &[], 1000).unwrap().chunks[0]
        );
    }

    #[test]
    fn an_inconsistent_mesh_is_the_cpu_renderers_error() {
        let mut mesh = Mesh::default();
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 0);
        triangle(&mut mesh, [Vec3::ZERO, Vec3::Y, Vec3::Z], 1);
        let mut broken = mesh.clone();
        broken.triangles[1] = [0, 1, 99];
        assert_eq!(
            pack(&broken, &FaceColors::new(), &[], 10),
            Err(GpuError::Render(RenderError::InvalidMesh { triangle: 1 }))
        );
        let mut short = mesh.clone();
        short.normals.pop();
        assert_eq!(
            pack(&short, &FaceColors::new(), &[], 10),
            Err(GpuError::Render(RenderError::InvalidMesh { triangle: 0 }))
        );
        let mut unlabelled = mesh.clone();
        unlabelled.face_ids.pop();
        assert!(matches!(
            pack(&unlabelled, &FaceColors::new(), &[], 10),
            Err(GpuError::Render(RenderError::InvalidMesh { .. }))
        ));
        assert_eq!(
            pack(&Mesh::default(), &FaceColors::new(), &[], 10)
                .unwrap()
                .triangles,
            0
        );
        assert!(
            pack(&Mesh::default(), &FaceColors::new(), &[], 0)
                .unwrap()
                .chunks
                .is_empty()
        );
    }

    #[test]
    fn hostile_coordinates_are_left_out_not_uploaded() {
        let mut mesh = Mesh::default();
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 0);
        triangle(
            &mut mesh,
            [Vec3::new(f64::NAN, 0.0, 0.0), Vec3::X, Vec3::Y],
            1,
        );
        triangle(
            &mut mesh,
            [Vec3::new(f64::INFINITY, 0.0, 0.0), Vec3::X, Vec3::Y],
            2,
        );
        // Finite in f64, infinite in f32.
        triangle(&mut mesh, [Vec3::new(1e300, 0.0, 0.0), Vec3::X, Vec3::Y], 3);
        // A normal that overflows counts as no normal.
        mesh.normals[0] = Vec3::new(1e300, 0.0, 0.0);
        let packed = pack(&mesh, &FaceColors::new(), &[], 10).unwrap();
        // Only the hostile triangles are left out, and the huge corner does
        // not move the centre: every drawn number is finite.
        for bytes in &packed.chunks {
            for vertex in bytes.as_chunks::<VERTEX_BYTES>().0 {
                assert!(floats(&vertex[..36]).iter().all(|v| v.is_finite()));
            }
        }
        assert_eq!(packed.triangles, 1);
        assert_eq!(packed.origin, Vec3::new(0.5, 0.5, 0.0));
    }

    /// A value per vertex goes with its corner, flagged; one that is not
    /// a number, or missing, is none.
    #[test]
    fn values_go_with_their_corners() {
        let mut mesh = Mesh::default();
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 0);
        let bytes = &pack(&mesh, &FaceColors::new(), &[2.5, f32::NAN], 1000)
            .unwrap()
            .chunks[0];
        let at =
            |corner: usize| floats(&bytes[corner * VERTEX_BYTES + 52..corner * VERTEX_BYTES + 60]);
        assert_eq!(at(0), [2.5, 1.0]);
        assert_eq!(at(1), [0.0, 0.0]);
        assert_eq!(at(2), [0.0, 0.0]);
    }
}
