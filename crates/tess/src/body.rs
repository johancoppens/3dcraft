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
//! a sphere but for holes whose loops do not go around its axis (the face
//! is then everything *outside* its loops, which all run clockwise: a
//! ball with a hole drilled through it across the sphere's axis);
//! reversed faces.
//!
//! Not handled, each an `Unsupported` error that names the face:
//! - a loop that winds around its surface more than once, or around both
//!   the axis and the tube of a torus (a Villarceau circle);
//! - a face around the axis of its surface that leaves no room for a
//!   straight seam between its winding loops, or between the poles of a
//!   sphere past its holes (16 places are tried).
//!
//! Refused as a `TessError::Face`, because the loops bound no region or
//! do not say which one is meant:
//! - a face that is everything *outside* its loops on a torus (a hole
//!   whose loop goes around neither the axis nor the tube), or on any
//!   surface that is not closed, where it would have no end: the loops
//!   run clockwise, as they do on an invalid face;
//! - a face on a freeform surface that closes on itself (a NURBS tube):
//!   `craft3d-geom` reports no period for it, so its seam cannot be
//!   followed;
//! - a whole closed surface given with its seam as an edge that the loop
//!   walks there and back.
//!
//! Faces are independent once the edges are points, so they are done on
//! several threads and put together in face order: the mesh does not
//! depend on the number of threads.
//!
//! The points of an edge are chosen for its curve alone, but a face may
//! need more of them: where another edge of the face comes nearer than
//! the chordal tolerance (the chords would cross it), and where the
//! surface's parameters distort the chord (near a pole). Faces say so
//! before they are triangulated, the chords they name are cut in two for
//! every face that uses the edge, and the faces concerned are done again:
//! a bounded number of rounds, each the same for any number of threads
//! (see `tessellate` below).

use crate::edge::{EdgeSamples, edge_points};
use crate::face::{Chord, FaceMesh, face_boundary, mesh_face};
use crate::{Mesh, TessError, TessParams};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
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

/// `job` for every index in `indices`, on up to `threads` threads; the
/// results in the order of `indices`.
fn in_order<T: Send>(
    indices: &[usize],
    threads: usize,
    job: impl Fn(usize) -> T + Sync,
) -> Result<Vec<T>, TessError> {
    let count = indices.len();
    let one = |n: usize| -> Option<T> { indices.get(n).map(|&index| job(index)) };
    let threads = threads.min(count);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        return Ok((0..count).filter_map(one).collect());
    }
    // Each worker takes the next index not yet taken. Which worker does
    // which varies from run to run; the results do not, and they are put
    // back in order.
    let next = AtomicUsize::new(0);
    let work = || -> Vec<(usize, T)> {
        let mut done = Vec::new();
        loop {
            let n = next.fetch_add(1, Ordering::Relaxed);
            let Some(result) = one(n) else {
                return done;
            };
            done.push((n, result));
        }
    };
    let mut slots: Vec<Option<T>> = (0..count).map(|_| None).collect();
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
        for (n, result) in done {
            if let Some(slot) = slots.get_mut(n) {
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

/// How often chords are cut in two for the sake of a face before the
/// faces are taken as they are. Every cut halves a chord, which quarters
/// how far it strays from its curve: 12 cuts are a factor of 1.7 × 10⁷,
/// from the coarsest tolerance that makes sense (a tenth of the body's
/// size) to below a modelling tolerance of 10⁻⁸ of it. Edges nearer to
/// each other than that are not apart.
const MAX_ROUNDS: usize = 12;

/// The faces that use one of `edges` (sorted), in face order.
fn faces_using(body: &Body, edges: &[u32]) -> Vec<usize> {
    let uses = |face: &craft3d_topo::Face| {
        face.loops
            .iter()
            .filter_map(|id| body.loop_(*id))
            .flat_map(|loop_| &loop_.coedges)
            .any(|coedge| edges.binary_search(&coedge.edge.0).is_ok())
    };
    (0..body.faces.len())
        .filter(|&index| body.faces.get(index).is_some_and(uses))
        .collect()
}

/// Whether a face can ask for finer chords at all: one on a plane, a
/// cylinder, a cone, a sphere or a torus whose edges are all straight
/// cannot (see `Boundary::chords_to_refine`).
fn can_ask(body: &Body, face: &craft3d_topo::Face) -> bool {
    matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_))
        || face
            .loops
            .iter()
            .filter_map(|id| body.loop_(*id))
            .flat_map(|loop_| &loop_.coedges)
            .filter_map(|coedge| body.edge(coedge.edge))
            .any(|edge| !matches!(edge.curve, Curve3::Line(_)))
}

/// The edges of `body` as points and, with `mesh`, its faces as triangles
/// over them.
///
/// The edges are first turned into points on their own. Then every face
/// says which chords it cannot do with (`Boundary::chords_to_refine`):
/// those are cut in two, and the faces that use the edges that changed
/// are looked at again, until no face asks for more. A face is
/// triangulated once it asks for nothing, and again when a neighbour had
/// one of its edges changed after that: both always use the same points.
///
/// Each round is a function of the body, the parameters and the points
/// of the round before, whatever the number of threads; faces are done in
/// parallel within a round and their wishes put together in face order.
/// After [`MAX_ROUNDS`] rounds, or when no chord asked for can be cut,
/// the faces are triangulated as they are: one whose boundary still
/// crosses itself is the error it always was.
fn tessellate(
    body: &Body,
    params: &TessParams,
    threads: usize,
    mesh: bool,
) -> Result<(EdgeSamples, Vec<Option<FaceResult>>), TessError> {
    let tolerance = params.chordal_tolerance;
    let mut edges = edge_points(body, params)?;
    let count = body.faces.len();
    let mut results: Vec<Option<FaceResult>> = (0..count).map(|_| None).collect();
    // The faces that still ask for finer chords.
    let mut asking = vec![false; count];
    // Without triangles to make, only the faces that may ask for
    // something are looked at.
    let mut todo: Vec<usize> = (0..count)
        .filter(|&index| mesh || body.faces.get(index).is_some_and(|f| can_ask(body, f)))
        .collect();
    let mut last = false;
    for round in 0..=MAX_ROUNDS {
        let points = &edges.points;
        let one = |index: usize| -> (Vec<Chord>, Option<FaceResult>) {
            let Some(face) = body.faces.get(index) else {
                return (Vec::new(), None);
            };
            let boundary = match face_boundary(body, points, face, index, params) {
                Ok(boundary) => boundary,
                Err(error) => return (Vec::new(), Some(Err(error))),
            };
            let wanted = boundary.chords_to_refine(body, tolerance);
            let result = (mesh && (last || wanted.is_empty()))
                .then(|| mesh_face(&boundary, face, index, params));
            (wanted, result)
        };
        let done = in_order(&todo, threads, one)?;
        let mut wanted: Vec<Chord> = Vec::new();
        for (&index, (more, result)) in todo.iter().zip(done) {
            if let Some(flag) = asking.get_mut(index) {
                *flag = !more.is_empty();
            }
            wanted.extend(more);
            if let Some(slot) = results.get_mut(index) {
                *slot = result;
            }
        }
        if last || wanted.is_empty() {
            break;
        }
        let changed = edges.split(body, &wanted, params);
        if changed.is_empty() && !mesh {
            break;
        }
        // Nothing left to cut, or enough tried: the faces that still ask
        // are taken as they are.
        last = changed.is_empty() || round + 1 >= MAX_ROUNDS;
        let users = faces_using(body, &changed);
        todo = (0..count)
            .filter(|index| {
                users.binary_search(index).is_ok() || asking.get(*index).copied().unwrap_or(false)
            })
            .collect();
    }
    Ok((edges, results))
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
/// number: given the points of the edges, faces are tessellated
/// independently, and they are merged in face order.
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
    let threads = if threads == 0 {
        automatic_threads()
    } else {
        threads
    };
    let (_, patches) = tessellate(body, params, threads, true)?;
    assemble(patches)
}

/// The faces' triangles as one mesh, in face order.
fn assemble(patches: Vec<Option<FaceResult>>) -> Result<Mesh, TessError> {
    let mut mesh = Mesh::default();
    for (index, patch) in patches.into_iter().enumerate() {
        let patch =
            patch.ok_or_else(|| TessError::Internal("a face was not tessellated".into()))??;
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

/// The edges of `body` as polylines: one per edge, in edge order, from the
/// edge's start vertex to its end vertex, within the chordal tolerance of
/// its curve. For a viewer that draws the edges of a body.
///
/// These are the very points [`tessellate_body`] puts along the edge for
/// every face that uses it, so the lines lie exactly on the borders of the
/// mesh's faces. That takes the boundaries of the faces (not their
/// triangles): where a face needs an edge finer than its curve does, the
/// edge is finer here too. A face that cannot be tessellated is no error
/// here. The result depends on the body and the parameters only.
pub fn tessellate_edges(body: &Body, params: &TessParams) -> Result<Vec<Vec<Vec3>>, TessError> {
    let tol = params.chordal_tolerance;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(TessError::InvalidTolerance(tol));
    }
    let (edges, _) = tessellate(body, params, automatic_threads(), false)?;
    Ok(edges.points)
}

/// What [`tessellate_body_and_edges`] returns.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshAndEdges {
    /// As from [`tessellate_body`]: the mesh, or the first face that
    /// cannot be tessellated.
    pub mesh: Result<Mesh, TessError>,
    /// As from [`tessellate_edges`].
    pub edges: Vec<Vec<Vec3>>,
}

/// [`tessellate_body`] and [`tessellate_edges`] in one go, for a viewer
/// that shows both: the boundaries of the faces are worked out once. The
/// edges come back also when a face cannot be tessellated.
pub fn tessellate_body_and_edges(
    body: &Body,
    params: &TessParams,
) -> Result<MeshAndEdges, TessError> {
    let tol = params.chordal_tolerance;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(TessError::InvalidTolerance(tol));
    }
    let (edges, patches) = tessellate(body, params, automatic_threads(), true)?;
    Ok(MeshAndEdges {
        mesh: assemble(patches),
        edges: edges.points,
    })
}
