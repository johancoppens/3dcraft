//! Tessellation of B-rep bodies, edges first: every edge is turned into
//! points once (`edge.rs`), and every face that uses the edge uses those
//! same points. Faces therefore meet without gaps or T-junctions, and
//! welding the mesh of a valid solid gives a closed manifold
//! (docs/architecture.md §6).
//!
//! Each face gets its own vertices, so it can carry its own normals; shared
//! positions are bit-identical and `Mesh::weld` joins them.
//!
//! One method serves every face (`face.rs`): the face's loops are followed
//! through the parameter space of its surface, the region they bound is
//! triangulated there (`cdt.rs`), and interior points are added until
//! every triangle is within the chordal tolerance of the surface.
//!
//! Handled: every kind of curve and surface of `craft3d-geom`; any number
//! of loops; loops that wind once around the axis of a cylinder, cone,
//! sphere or torus, or once around the tube of a torus; faces that reach
//! a pole, an apex or a collapsed edge of a patch; whole spheres and tori;
//! reversed faces.
//!
//! Not handled, each an `Unsupported` error that names the face:
//! - a loop that winds around its surface more than once, or around both
//!   the axis and the tube of a torus (a Villarceau circle);
//! - a face around the axis of its surface that leaves no room for a
//!   straight seam between its winding loops (16 places are tried).
//!
//! Refused as a `TessError::Face`, because the loops bound no region or
//! do not say which one is meant:
//! - a face that is everything *outside* its loops on a closed surface (a
//!   sphere with one hole whose loop does not go around the axis): the
//!   loops then run clockwise, as they would on an invalid face;
//! - a face on a freeform surface that closes on itself (a NURBS tube):
//!   `craft3d-geom` reports no period for it, so its seam cannot be
//!   followed;
//! - a whole closed surface given with its seam as an edge that the loop
//!   walks there and back.
//!
//! Faces are independent once the edges are points, so they are done on
//! several threads and put together in face order: the mesh does not
//! depend on the number of threads.

use crate::edge::edge_points;
use crate::face::{FaceMesh, tessellate_face};
use crate::{Mesh, TessError, TessParams};
use craft3d_topo::Body;
use std::sync::atomic::{AtomicUsize, Ordering};

/// No more threads than this by default; faces are small jobs.
#[cfg(not(target_arch = "wasm32"))]
const MAX_AUTOMATIC_THREADS: usize = 16;

fn automatic_threads() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(MAX_AUTOMATIC_THREADS)
    }
}

type FaceResult = Result<FaceMesh, TessError>;

/// Every face of `body`, on up to `threads` threads, in face order.
fn all_faces(
    body: &Body,
    edges: &[Vec<craft3d_math::Vec3>],
    params: &TessParams,
    threads: usize,
) -> Result<Vec<FaceResult>, TessError> {
    let count = body.faces.len();
    let one = |index: usize| -> Option<FaceResult> {
        let face = body.faces.get(index)?;
        Some(tessellate_face(body, edges, face, index, params))
    };
    let threads = threads.min(count);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        return Ok((0..count).filter_map(one).collect());
    }
    // Each worker takes the next face not yet taken. Which worker does
    // which face varies from run to run; the results do not, and they are
    // put back in face order.
    let next = AtomicUsize::new(0);
    let work = || -> Vec<(usize, FaceResult)> {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(result) = one(index) else {
                return done;
            };
            done.push((index, result));
        }
    };
    let mut slots: Vec<Option<FaceResult>> = (0..count).map(|_| None).collect();
    let mut panicked = false;
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 1..threads {
            // A thread that cannot be started is not an error: the others
            // (at least this one) do its share.
            if let Ok(worker) = std::thread::Builder::new().spawn_scoped(scope, work) {
                workers.push(worker);
            }
        }
        let mut done = work();
        for worker in workers {
            match worker.join() {
                Ok(more) => done.extend(more),
                Err(_) => panicked = true,
            }
        }
        for (index, result) in done {
            if let Some(slot) = slots.get_mut(index) {
                *slot = Some(result);
            }
        }
    });
    if panicked {
        return Err(TessError::Internal(
            "a tessellation thread stopped unexpectedly".into(),
        ));
    }
    slots
        .into_iter()
        .map(|slot| slot.ok_or_else(|| TessError::Internal("a face was not tessellated".into())))
        .collect()
}

/// Tessellates every face of `body`. Triangle `face_ids` are face indices;
/// triangles face outward. The body should pass `craft3d_topo::check`.
///
/// Uses as many threads as the machine offers (up to 16; one on `wasm32`);
/// see [`tessellate_body_with_threads`]. The result does not depend on it.
pub fn tessellate_body(body: &Body, params: &TessParams) -> Result<Mesh, TessError> {
    tessellate_body_with_threads(body, params, 0)
}

/// [`tessellate_body`] on a given number of threads; 0 means "as many as
/// the machine offers". The mesh is the same, bit for bit, whatever the
/// number: faces are tessellated independently and merged in face order.
/// When several faces fail, the error is that of the first of them.
pub fn tessellate_body_with_threads(
    body: &Body,
    params: &TessParams,
    threads: usize,
) -> Result<Mesh, TessError> {
    let tol = params.chordal_tolerance;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(TessError::InvalidTolerance(tol));
    }
    let edges = edge_points(body, params)?;
    let threads = if threads == 0 {
        automatic_threads()
    } else {
        threads
    };
    let mut mesh = Mesh::default();
    for (index, patch) in all_faces(body, &edges, params, threads)?
        .into_iter()
        .enumerate()
    {
        let patch = patch?;
        let face_id = u32::try_from(index).map_err(|_| TessError::TooManyVertices)?;
        let base = u32::try_from(mesh.positions.len()).map_err(|_| TessError::TooManyVertices)?;
        if u64::from(base) + patch.positions.len() as u64 > u64::from(u32::MAX) {
            return Err(TessError::TooManyVertices);
        }
        for [a, b, c] in patch.triangles {
            mesh.triangles.push([base + a, base + b, base + c]);
            mesh.face_ids.push(face_id);
        }
        mesh.positions.extend(patch.positions);
        mesh.normals.extend(patch.normals);
    }
    Ok(mesh)
}
