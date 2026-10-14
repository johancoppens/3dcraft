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
//! - a face around the axis of its surface for which no seam is found
//!   between its winding loops (or the ends of the surface), neither a
//!   straight one nor one in pieces through its other loops. A face
//!   whose loops do not cross has one; this is what is left for loops
//!   that do, after the chords were made finer.
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
//! surface's parameters distort the chord (near a pole), and where the
//! face is so narrow that its loops, as polygons, enclose nothing (a
//! sliver between a line and an arc flatter than the tolerance). Faces say so
//! before they are triangulated, the chords they name are cut in two for
//! every face that uses the edge, and the faces concerned are done again:
//! a bounded number of rounds, each the same for any number of threads
//! (see `tessellate` below).

use crate::edge::{EdgeSamples, edge_points};
use crate::face::{Chord, FaceMesh, curved_chords, face_boundary, mesh_face};
use crate::{Mesh, TessError, TessParams};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
use craft3d_topo::Body;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// A point as the key it is welded by: its bits, with −0 as 0.
fn place(p: Vec3) -> [u64; 3] {
    [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f64::to_bits)
}

/// One use of a segment between two points: by a triangle of the face's
/// mesh, or (with its chord) by an edge of the face as its loops run.
struct Use {
    /// The segment's ends, the smaller key first.
    ends: ([u64; 3], [u64; 3]),
    /// Whether it runs from the first of `ends` to the second.
    along: bool,
    chord: Option<Chord>,
}

impl Use {
    /// None for a segment without length.
    fn new(from: Vec3, to: Vec3, chord: Option<Chord>) -> Option<Self> {
        let (from, to) = (place(from), place(to));
        match from.cmp(&to) {
            std::cmp::Ordering::Less => Some(Self {
                ends: (from, to),
                along: true,
                chord,
            }),
            std::cmp::Ordering::Greater => Some(Self {
                ends: (to, from),
                along: false,
                chord,
            }),
            std::cmp::Ordering::Equal => None,
        }
    }
}

/// Holds the mesh of a face against the face's edges: the promise that
/// the mesh of a closed solid is closed, checked where it can break.
///
/// `border` has every side of a triangle that lies on the boundary of the
/// face's region, directed as the triangle runs. The loops of the face
/// say what must be there: every chord of every edge, once for each time
/// a loop uses the edge and in that direction (a loop runs
/// counter-clockwise about the outward normal, as the triangles do). The
/// two must be the same, point for point and bit for bit, and what the
/// mesh has beyond the edges (the two sides of a seam the tessellator cut
/// itself) must be there once each way. Faces that pass and share an
/// edge use its chords in opposite directions, so the welded mesh of a
/// body whose every edge is used once each way is closed, without
/// welding it: the sides of triangles inside a face are shared by two of
/// them by the making of the triangulation.
///
/// The cost is that of sorting the boundary of the face, not its
/// triangles.
fn closes_along_edges(
    body: &Body,
    points: &[Vec<Vec3>],
    face: &craft3d_topo::Face,
    index: usize,
    border: &[(Vec3, Vec3)],
) -> Result<(), TessError> {
    let mut uses: Vec<Use> = border
        .iter()
        .filter_map(|&(from, to)| Use::new(from, to, None))
        .collect();
    let coedges = face
        .loops
        .iter()
        .filter_map(|id| body.loop_(*id))
        .flat_map(|loop_| &loop_.coedges);
    for coedge in coedges {
        let Some(edge) = points.get(coedge.edge.0 as usize) else {
            continue;
        };
        for (k, (from, to)) in edge.iter().zip(edge.iter().skip(1)).enumerate() {
            let chord = Some((coedge.edge.0, u32::try_from(k).unwrap_or(u32::MAX)));
            let (from, to) = if coedge.reversed {
                (*to, *from)
            } else {
                (*from, *to)
            };
            uses.extend(Use::new(from, to, chord));
        }
    }
    uses.sort_by_key(|x| x.ends);
    for group in uses.chunk_by(|x, y| x.ends == y.ends) {
        // How often the mesh and the edges have the segment, each way.
        let count = |of_edge: bool, along: bool| {
            group
                .iter()
                .filter(|u| u.chord.is_some() == of_edge && u.along == along)
                .count()
        };
        let mesh = (count(false, true), count(false, false));
        let edges = (count(true, true), count(true, false));
        let chord = group.iter().find_map(|u| u.chord);
        let reason = match chord {
            Some(_) if mesh == edges => continue,
            None if mesh == (1, 1) => continue,
            Some((edge, k)) => {
                let other = body.faces.iter().enumerate().position(|(n, other)| {
                    n != index
                        && other
                            .loops
                            .iter()
                            .filter_map(|id| body.loop_(*id))
                            .flat_map(|loop_| &loop_.coedges)
                            .any(|coedge| coedge.edge.0 == edge)
                });
                let beyond = match other {
                    Some(other) => format!("which it shares with face {other}"),
                    None => "which no other face uses".to_string(),
                };
                format!(
                    "its mesh is not closed along edge {edge}, {beyond}: chord {k} of the edge \
                     is the side of {} of its triangles where its loops use the edge {} times",
                    mesh.0 + mesh.1,
                    edges.0 + edges.1
                )
            }
            None => "its mesh is not closed: it has a border where the face has no edge (a \
                     seam whose two sides do not meet)"
                .to_string(),
        };
        return Err(TessError::Face {
            face: index,
            reason,
        });
    }
    Ok(())
}

/// The triangles of a face's result: none for a face that failed or that
/// was not triangulated.
fn triangles_of(result: Option<&FaceResult>) -> usize {
    match result {
        Some(Ok(mesh)) => mesh.triangles.len(),
        Some(Err(_)) | None => 0,
    }
}

/// The triangle budget within one round of [`tessellate`].
///
/// A round is over the budget when the triangles of all faces at its end
/// (those the round did not touch, and the round's own) are more than the
/// budget: a function of the body, the parameters and the points of the
/// round before. The faces of a round are done in parallel, and each adds
/// its triangles to a count as it finishes; once that count is over the
/// budget the round's verdict is certain (the faces still to come only
/// add), so they are skipped. Which faces were skipped depends on timing;
/// that the round is over the budget does not, and neither does what is
/// reported: the first faces, in face order, whose triangles together
/// exceed the budget, as a single thread finds them (a face among them
/// that was skipped is done after all).
struct Budget {
    max_triangles: usize,
    /// The triangles of the faces this round leaves alone, then also of
    /// its faces that are done.
    triangles: AtomicUsize,
    over: AtomicBool,
}

impl Budget {
    /// For a round that redoes the faces `todo` (sorted) of `results`.
    fn for_round(max_triangles: usize, results: &[Option<FaceResult>], todo: &[usize]) -> Self {
        let kept = results
            .iter()
            .enumerate()
            .filter(|(index, _)| todo.binary_search(index).is_err())
            .fold(0_usize, |sum, (_, result)| {
                sum.saturating_add(triangles_of(result.as_ref()))
            });
        Self {
            max_triangles,
            triangles: AtomicUsize::new(kept),
            over: AtomicBool::new(kept > max_triangles),
        }
    }

    /// True once the round is known to be over the budget: what is not
    /// started yet need not be.
    fn spent(&self) -> bool {
        self.over.load(Ordering::Relaxed)
    }

    /// Counts a face that is done.
    fn add(&self, result: Option<&FaceResult>) {
        let more = triangles_of(result);
        // No wrapping to fear: every triangle counted is in memory.
        let before = self.triangles.fetch_add(more, Ordering::Relaxed);
        if before.saturating_add(more) > self.max_triangles {
            self.over.store(true, Ordering::Relaxed);
        }
    }
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
///
/// With a triangle budget (`TessParams::max_triangles`, when `mesh`), the
/// first round at whose end the faces have more triangles than that ends
/// the work with `TessError::OverBudget`, and ends itself as soon as that
/// is certain (see [`Budget`]). It goes before the error of a face. When
/// no round is over the budget, nothing is different.
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
        let budget = params
            .max_triangles
            .filter(|_| mesh)
            .map(|max| Budget::for_round(max, &results, &todo));
        let budget = budget.as_ref();
        let face_round = |index: usize| -> (Vec<Chord>, Option<FaceResult>) {
            let Some(face) = body.faces.get(index) else {
                return (Vec::new(), None);
            };
            let mut starved = false;
            let boundary = match face_boundary(body, points, face, index, params, &mut starved) {
                Ok(boundary) => boundary,
                Err(error) => {
                    // Loops that enclose nothing as polygons: finer chords
                    // may give them something to enclose. All the face's
                    // curved chords are asked for; the error stands when
                    // there are none, and in the last round.
                    let wanted = if starved && !last {
                        curved_chords(body, points, face)
                    } else {
                        Vec::new()
                    };
                    let result = wanted.is_empty().then_some(Err(error));
                    return (wanted, result);
                }
            };
            let wanted = boundary.chords_to_refine(body, tolerance);
            let result = (mesh && (last || wanted.is_empty())).then(|| {
                let mut made = mesh_face(&boundary, face, index, params)?;
                closes_along_edges(body, points, face, index, &made.border)?;
                made.border = Vec::new();
                Ok(made)
            });
            (wanted, result)
        };
        // Whether the face was looked at, what it asks for and its mesh.
        let one = |index: usize| -> (bool, Vec<Chord>, Option<FaceResult>) {
            if budget.is_some_and(Budget::spent) {
                return (false, Vec::new(), None);
            }
            let (wanted, result) = face_round(index);
            if let Some(budget) = budget {
                budget.add(result.as_ref());
            }
            (true, wanted, result)
        };
        let done = in_order(&todo, threads, one)?;
        if let Some(budget) = budget.filter(|budget| budget.spent()) {
            // In face order: the faces the round left alone as they were,
            // its own as they are now, and a skipped one done after all.
            let mut redone = todo.iter().zip(&done).peekable();
            let (mut triangles, mut faces) = (0_usize, 0_usize);
            for (index, kept) in results.iter().enumerate() {
                let count = match redone.next_if(|(at, _)| **at == index) {
                    Some((_, (true, _, result))) => triangles_of(result.as_ref()),
                    Some((_, (false, ..))) => triangles_of(face_round(index).1.as_ref()),
                    None => triangles_of(kept.as_ref()),
                };
                triangles = triangles.saturating_add(count);
                faces = faces.saturating_add(1);
                if triangles > budget.max_triangles {
                    break;
                }
            }
            return Err(TessError::OverBudget {
                triangles,
                faces,
                max_triangles: budget.max_triangles,
            });
        }
        let mut wanted: Vec<Chord> = Vec::new();
        for (&index, (_, more, result)) in todo.iter().zip(done) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Frame;
    use craft3d_testkit::solids;

    /// The border the mesh of face `index` must have: the chords of its
    /// edges as its loops run.
    fn border_of(body: &Body, points: &[Vec<Vec3>], index: usize) -> Vec<(Vec3, Vec3)> {
        let mut border = Vec::new();
        for id in &body.faces[index].loops {
            for coedge in &body.loop_(*id).unwrap().coedges {
                let edge = &points[coedge.edge.0 as usize];
                for pair in edge.windows(2) {
                    border.push(if coedge.reversed {
                        (pair[1], pair[0])
                    } else {
                        (pair[0], pair[1])
                    });
                }
            }
        }
        border
    }

    fn reason(result: Result<(), TessError>) -> String {
        match result {
            Err(TessError::Face { face: 1, reason }) => reason,
            other => panic!("expected an error at face 1, got {other:?}"),
        }
    }

    /// The check that holds a face's mesh against its edges says what is
    /// wrong with a border that is not the one the loops ask for. (No
    /// face of the suites has such a mesh: the borders here are made up.)
    #[test]
    fn a_mesh_that_does_not_close_along_an_edge_is_named() {
        let body = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
        let params = TessParams {
            chordal_tolerance: 1e-2,
            ..TessParams::default()
        };
        let points = edge_points(&body, &params).unwrap().points;
        let face = &body.faces[1];
        let whole = border_of(&body, &points, 1);
        assert!(whole.len() > 8);
        assert_eq!(closes_along_edges(&body, &points, face, 1, &whole), Ok(()));
        // In any order.
        let mut shuffled = whole.clone();
        shuffled.reverse();
        shuffled.swap(1, 5);
        assert_eq!(
            closes_along_edges(&body, &points, face, 1, &shuffled),
            Ok(())
        );

        // A chord no triangle ends on: the mesh is open there.
        let mut open = whole.clone();
        open.remove(3);
        let why = reason(closes_along_edges(&body, &points, face, 1, &open));
        assert!(why.contains("not closed along edge"), "{why}");
        assert!(why.contains("shares with face 0"), "{why}");
        assert!(why.contains("the side of 0 of its triangles"), "{why}");
        // A chord two triangles end on, and one the wrong way round.
        let mut twice = whole.clone();
        twice.push(whole[3]);
        let why = reason(closes_along_edges(&body, &points, face, 1, &twice));
        assert!(why.contains("the side of 2 of its triangles"), "{why}");
        let mut backwards = whole.clone();
        backwards[3] = (whole[3].1, whole[3].0);
        let why = reason(closes_along_edges(&body, &points, face, 1, &backwards));
        assert!(why.contains("not closed along edge"), "{why}");
        // A side that is no chord of any edge, used once: a seam whose
        // other side is missing, or a chord with a point that is not the
        // edge's.
        let (from, to) = whole[3];
        let mut stray = whole.clone();
        stray.push((from, Vec3::new(0.0, 0.0, 1.0)));
        let why = reason(closes_along_edges(&body, &points, face, 1, &stray));
        assert!(why.contains("where the face has no edge"), "{why}");
        let mut moved = whole.clone();
        moved[3] = (from, to + Vec3::new(1e-12, 0.0, 0.0));
        assert!(closes_along_edges(&body, &points, face, 1, &moved).is_err());
        // Both sides of a seam cancel; a side without length is nothing.
        let mut seamed = whole.clone();
        seamed.push((from, Vec3::new(0.0, 0.0, 1.0)));
        seamed.push((Vec3::new(0.0, 0.0, 1.0), from));
        seamed.push((to, to));
        assert_eq!(closes_along_edges(&body, &points, face, 1, &seamed), Ok(()));
        // −0 is 0.
        let negative =
            |p: Vec3| Vec3::new(p.x + 0.0, p.y + 0.0, if p.z == 0.0 { -0.0 } else { p.z });
        let signed: Vec<(Vec3, Vec3)> = whole
            .iter()
            .map(|&(a, b)| (negative(a), negative(b)))
            .collect();
        assert_eq!(closes_along_edges(&body, &points, face, 1, &signed), Ok(()));
    }
}
