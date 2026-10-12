//! A mesh as the GPU wants it: three unshared vertices per triangle in
//! single precision, relative to the centre of the mesh.

use crate::GpuError;
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::RenderError;
use craft3d_tess::Mesh;
use std::sync::Arc;

/// Largest accepted number of triangles (2 GB of vertex data).
pub const MAX_TRIANGLES: usize = 1 << 24;

/// Position, normal, the triangle's own normal (3 × `f32` each) and the face
/// id (`u32`).
pub(crate) const VERTEX_BYTES: usize = 40;
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
pub(crate) fn pack(mesh: &Mesh, chunk_triangles: usize) -> Result<Packed, GpuError> {
    if mesh.normals.len() != mesh.positions.len() || mesh.face_ids.len() != mesh.triangles.len() {
        return Err(RenderError::InvalidMesh { triangle: 0 }.into());
    }
    if mesh.triangles.len() > MAX_TRIANGLES {
        return Err(GpuError::MeshTooLarge {
            triangles: mesh.triangles.len(),
        });
    }
    let chunk_triangles = chunk_triangles.clamp(1, CHUNK_TRIANGLES);
    // The centre of the positions that fit in `f32` at all, so that one
    // hostile coordinate cannot push the rest of the mesh out of range.
    let representable = mesh
        .positions
        .iter()
        .copied()
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
        for (position, (world, normal)) in [a, b, c].into_iter().zip(corners) {
            push(&mut current, position);
            // A normal that cannot be narrowed counts as "no normal".
            push(&mut current, narrow(normal).unwrap_or([0.0; 3]));
            push(&mut current, flat);
            current.extend_from_slice(&face.to_le_bytes());
            packed.bounds.add(world);
        }
        packed.triangles += 1;
    }
    if !current.is_empty() {
        packed.chunks.push(current);
    }
    Ok(packed)
}

pub(crate) struct Chunk {
    pub buffer: wgpu::Buffer,
    pub vertices: u32,
}

/// A mesh on the GPU, made by [`crate::Renderer::upload`]. It belongs to the
/// renderer that made it: another renderer refuses to draw it.
pub struct GpuMesh {
    pub(crate) chunks: Vec<Chunk>,
    pub(crate) origin: Vec3,
    pub(crate) bounds: Bbox3,
    triangles: usize,
    /// The token of the renderer that made this mesh.
    pub(crate) owner: Arc<()>,
}

impl GpuMesh {
    /// Number of triangles that are drawn (those of the mesh, less the ones
    /// with a corner the GPU cannot represent).
    pub fn triangle_count(&self) -> usize {
        self.triangles
    }

    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &Mesh,
        owner: &Arc<()>,
    ) -> Result<GpuMesh, GpuError> {
        let by_limit = device.limits().max_buffer_size / TRIANGLE_BYTES as u64;
        let per_chunk = usize::try_from(by_limit).unwrap_or(usize::MAX);
        if per_chunk == 0 {
            return Err(GpuError::Device(
                "the device's buffers are too small for one triangle".to_string(),
            ));
        }
        let packed = pack(mesh, per_chunk)?;
        let mut chunks = Vec::with_capacity(packed.chunks.len());
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
            chunks.push(Chunk { buffer, vertices });
        }
        Ok(GpuMesh {
            chunks,
            origin: packed.origin,
            bounds: packed.bounds,
            triangles: packed.triangles,
            owner: Arc::clone(owner),
        })
    }
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
        let packed = pack(&mesh, 1000).unwrap();
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
        let second = floats(&bytes[40..52]);
        assert_eq!(second, [1.0, -2.0, 0.0]);
        let (lo, hi) = packed.bounds.corners().unwrap();
        assert_eq!((lo, hi), (far, far + Vec3::new(2.0, 4.0, 0.0)));
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
        let packed = pack(&mesh, 3).unwrap();
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
        assert_eq!(packed.chunks.concat(), pack(&mesh, 1000).unwrap().chunks[0]);
    }

    #[test]
    fn an_inconsistent_mesh_is_the_cpu_renderers_error() {
        let mut mesh = Mesh::default();
        triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Y], 0);
        triangle(&mut mesh, [Vec3::ZERO, Vec3::Y, Vec3::Z], 1);
        let mut broken = mesh.clone();
        broken.triangles[1] = [0, 1, 99];
        assert_eq!(
            pack(&broken, 10),
            Err(GpuError::Render(RenderError::InvalidMesh { triangle: 1 }))
        );
        let mut short = mesh.clone();
        short.normals.pop();
        assert_eq!(
            pack(&short, 10),
            Err(GpuError::Render(RenderError::InvalidMesh { triangle: 0 }))
        );
        let mut unlabelled = mesh.clone();
        unlabelled.face_ids.pop();
        assert!(matches!(
            pack(&unlabelled, 10),
            Err(GpuError::Render(RenderError::InvalidMesh { .. }))
        ));
        assert_eq!(pack(&Mesh::default(), 10).unwrap().triangles, 0);
        assert!(pack(&Mesh::default(), 0).unwrap().chunks.is_empty());
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
        let packed = pack(&mesh, 10).unwrap();
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
}
