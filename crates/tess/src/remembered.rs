//! Meshes of faces made lately, by everything a face's mesh is made from.
//!
//! The check of a body against itself after every feature meshes the
//! faces near what the feature changed, and most of them are what they
//! were the feature before: the same surface, the same edges with the same
//! points along them. A face's mesh (in a round where it asks for no finer
//! chords) is a function of its surface and sense, of the points, curves
//! and order of the edges around it and of the parameters of the mesh;
//! written out, every bit of them is the key, and a face found by its key
//! is given the mesh made for it before, the same bits. What is kept
//! changes no mesh, only the work.

use crate::TessParams;
use crate::face::FaceMesh;
use craft3d_math::{Bits, Vec3};
use craft3d_topo::{Body, Face};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

/// How many bytes the meshes kept take at most, keys included; the
/// oldest go first.
#[cfg(not(target_arch = "wasm32"))]
const BYTES: usize = 64 << 20;
#[cfg(target_arch = "wasm32")]
const BYTES: usize = 8 << 20;

#[derive(Default)]
struct Kept {
    /// Each mesh with the work making it took (`craft3d_geom::allowance`,
    /// on the thread that made it), which finding it charges again.
    meshes: BTreeMap<Vec<u64>, (Arc<FaceMesh>, u64)>,
    order: VecDeque<Vec<u64>>,
    bytes: usize,
}

static KEPT: Mutex<Kept> = Mutex::new(Kept {
    meshes: BTreeMap::new(),
    order: VecDeque::new(),
    bytes: 0,
});

/// About what a mesh takes, its key with it.
fn size_of(key: &[u64], mesh: &FaceMesh) -> usize {
    use std::mem::size_of;
    std::mem::size_of_val(key)
        + (mesh.positions.len() + mesh.normals.len()) * size_of::<Vec3>()
        + mesh.triangles.len() * size_of::<[u32; 3]>()
        + (mesh.border.len() + mesh.rungs.len()) * size_of::<(Vec3, Vec3)>()
}

/// Everything the mesh of face `face` of `body` is made from, written out:
/// the parameters, whether this is the last round, the surface and its
/// sense, and per loop per coedge its direction, its edge's curve, the
/// points along the edge and whether the loop closes. `None` where the
/// face refers to what the body does not have.
pub(crate) fn key(
    body: &Body,
    points: &[Vec<Vec3>],
    face: &Face,
    params: &TessParams,
    last: bool,
) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    params.chordal_tolerance.push_bits(&mut out);
    out.push(u64::from(params.max_segments));
    match params.max_triangles {
        Some(most) => out.extend([1, u64::try_from(most).unwrap_or(u64::MAX)]),
        None => out.push(0),
    }
    match params.max_angle {
        Some(angle) => out.extend([1, angle.to_bits()]),
        None => out.push(0),
    }
    out.push(u64::from(last));
    face.surface.push_bits(&mut out);
    out.push(u64::from(face.reversed));
    out.push(face.loops.len() as u64);
    for loop_id in &face.loops {
        let loop_ = body.loop_(*loop_id)?;
        out.push(loop_.coedges.len() as u64);
        let mut ends = Vec::with_capacity(loop_.coedges.len());
        for coedge in &loop_.coedges {
            let edge = body.edges.get(coedge.edge.0 as usize)?;
            ends.push(body.coedge_ends(*coedge)?);
            out.push(u64::from(coedge.reversed));
            edge.curve.push_bits(&mut out);
            let along = points.get(coedge.edge.0 as usize)?;
            out.push(along.len() as u64);
            for p in along {
                p.push_bits(&mut out);
            }
        }
        let closed = ends
            .iter()
            .zip(ends.iter().cycle().skip(1))
            .all(|(this, next)| this.1 == next.0);
        out.push(u64::from(closed));
    }
    Some(out)
}

/// The mesh kept for `key`, if any.
pub(crate) fn find(key: &[u64]) -> Option<Arc<FaceMesh>> {
    let (mesh, units) = KEPT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .meshes
        .get(key)
        .cloned()?;
    craft3d_geom::allowance::charge(units);
    Some(mesh)
}

/// Keeps `mesh` for `key`, letting go of the oldest while there is not
/// room.
pub(crate) fn keep(key: Vec<u64>, mesh: &FaceMesh, units: u64) {
    let bytes = size_of(&key, mesh);
    if bytes > BYTES / 4 {
        return;
    }
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.meshes.contains_key(&key) {
        return;
    }
    while kept.bytes.saturating_add(bytes) > BYTES {
        let Some(oldest) = kept.order.pop_front() else {
            break;
        };
        if let Some((gone, _)) = kept.meshes.remove(&oldest) {
            kept.bytes = kept.bytes.saturating_sub(size_of(&oldest, &gone));
        }
    }
    kept.bytes = kept.bytes.saturating_add(bytes);
    kept.order.push_back(key.clone());
    kept.meshes.insert(key, (Arc::new(mesh.clone()), units));
}

/// How many meshes are kept and about how many bytes they take.
pub fn kept_meshes() -> (usize, usize) {
    let kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    (kept.meshes.len(), kept.bytes)
}

/// Lets go of every mesh kept (they are made again, the same, when asked
/// for).
pub fn forget_kept_meshes() {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    *kept = Kept::default();
}
