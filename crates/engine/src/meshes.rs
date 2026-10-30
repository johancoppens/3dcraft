//! The meshes of bodies, kept by the body each is of, so that a scene
//! rebuilt after an edit meshes only the bodies the edit changed
//! (docs/design/tessellator.md › *A scene rebuild meshes only what
//! changed*); and with them what else a scene says of a body (the names
//! of its faces, edges and vertices as text, which faces each edge lies
//! between), worked out once per body too ([`BodyFacts`]).
//!
//! A scene is the mesh of every body of the document, and a viewer asks
//! for a new one after every edit. An edit changes a few bodies;
//! regeneration hands on the same `Arc<Body>` for every body it did not
//! touch (`craft3d_regen::RegenCache`), and a body is immutable once made.
//! The tessellator is a function of the body and its parameters, on any
//! number of threads, so the mesh of a body is a function of that `Arc`
//! and of how it is sampled: each is kept here, by the `Arc`'s address
//! (held by a `Weak`, so that no other body can be where it was while the
//! entry exists) and the sampling, within a byte budget, the least
//! recently used first out. Nothing in a scene depends on whether a mesh
//! was found here or made now: the scene is the same, bit for bit, and
//! only the time differs (proved by `crates/engine/tests/scene_reuse.rs`).
//!
//! One store serves every scene of a session, on whatever thread it is
//! built (the display scene is refined beside the engine thread,
//! `apps/craft3d/src/refine.rs`). The lock is held to look up and to
//! keep, never while meshing.

use craft3d_math::{Transform, Vec3};
use craft3d_regen::Names;
use craft3d_tess::{
    Mesh, TessError, TessParams, tessellate_body_and_edges, tessellate_within_budget,
};
use craft3d_topo::Body;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// A body's edges as polylines, one per edge in the order of the B-rep
/// (`craft3d_tess::tessellate_edges`).
pub type EdgePolylines = Vec<Vec<Vec3>>;

/// A body meshed: shared, so that the scenes of several states of a
/// document hold one copy of the mesh of a body they have in common.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BodyMesh {
    pub mesh: Arc<Mesh>,
    pub edges: Arc<EdgePolylines>,
    /// True when the body was meshed at a coarser tolerance than asked
    /// for, to stay within what the tessellator can do or within its own
    /// budget ([`Limit::Own`]).
    pub coarsened: bool,
    /// For a body placed as a rigid copy of another ([`note_placed`]):
    /// the mesh of the body it was copied from, and the motion that
    /// takes that mesh to this one. A viewer that has the source's mesh
    /// uploaded moves it, it does not upload this one.
    pub placed: Option<(Arc<Mesh>, Transform)>,
}

/// How deep a chain of placed bodies is followed (a body placed from a
/// body placed from another, as subassemblies nest): far deeper than any
/// assembly nests; beyond it a body is meshed itself.
const PLACED_DEPTH: usize = 32;

/// A body made as a rigid copy of another (an assembly's instance placed
/// by its pose, `asm::build_shown`): the copy, held by a `Weak` (so no
/// other body can be where it is while the entry exists), the body it was
/// copied from, and the rigid motion.
struct Placement {
    placed: Weak<Body>,
    source: Arc<Body>,
    pose: Transform,
}

/// Every placed body still alive, by the address of its `Arc`: a fact of
/// the body, true for as long as it lives, whichever session meshes it.
static PLACED: Mutex<BTreeMap<usize, Placement>> = Mutex::new(BTreeMap::new());

/// How many placements were kept when the dead ones were last let go.
static PLACED_PRUNED: Mutex<usize> = Mutex::new(0);

/// Notes that `placed` is `source` moved by `pose`, so that its mesh, its
/// edges and its facts are those of `source` moved by `pose`: the source
/// is meshed once, and every moment of a motion moves that mesh instead
/// of meshing a new body (docs/design/motion.md › *Assemblies*). Only a
/// rigid motion that does not mirror is noted (a mirror turns the
/// triangles inside out); a body not noted is meshed itself. The scene
/// of a placed body does not depend on whether a mesh was found in the
/// store or made now: it is always the moved mesh of its source.
pub(crate) fn note_placed(placed: &Arc<Body>, source: &Arc<Body>, pose: &Transform) {
    if !(pose.is_isometry(1e-9) && pose.determinant() > 0.0) {
        return;
    }
    let mut map = PLACED.lock().unwrap_or_else(PoisonError::into_inner);
    let mut pruned = PLACED_PRUNED.lock().unwrap_or_else(PoisonError::into_inner);
    if map.len() > pruned.saturating_mul(2).saturating_add(64) {
        map.retain(|_, p| p.placed.strong_count() > 0);
        *pruned = map.len();
    }
    map.insert(
        Arc::as_ptr(placed) as usize,
        Placement {
            placed: Arc::downgrade(placed),
            source: Arc::clone(source),
            pose: *pose,
        },
    );
}

/// The body `body` was placed from and the motion, when it was placed
/// ([`note_placed`]).
fn placed_of(body: &Arc<Body>) -> Option<(Arc<Body>, Transform)> {
    let map = PLACED.lock().unwrap_or_else(PoisonError::into_inner);
    let found = map.get(&(Arc::as_ptr(body) as usize))?;
    std::ptr::eq(found.placed.as_ptr(), Arc::as_ptr(body))
        .then(|| (Arc::clone(&found.source), found.pose))
}

/// The body a placed body comes from at the end of its chain, and the
/// motion from there to it; `None` for a body that was not placed.
fn root_of(body: &Arc<Body>) -> Option<(Arc<Body>, Transform)> {
    let (mut source, mut pose) = placed_of(body)?;
    for _ in 0..PLACED_DEPTH {
        let Some((further, first)) = placed_of(&source) else {
            return Some((source, pose));
        };
        // `first` takes `further` to `source`, `pose` takes `source` on.
        pose = first.then(&pose);
        source = further;
    }
    None
}

/// The body whose measures (its area) `body` has: the body it was placed
/// from at the end of its chain when it was placed by a rigid motion
/// ([`note_placed`]), else itself.
pub(crate) fn measured_as(body: &Arc<Body>) -> Arc<Body> {
    root_of(body).map_or_else(|| Arc::clone(body), |(source, _)| source)
}

/// `mesh` moved by the rigid motion `pose`: its positions and normals.
fn moved_mesh(mesh: &Mesh, pose: &Transform) -> Mesh {
    Mesh {
        positions: mesh.positions.iter().map(|&p| pose.point(p)).collect(),
        normals: mesh.normals.iter().map(|&n| pose.vector(n)).collect(),
        triangles: mesh.triangles.clone(),
        face_ids: mesh.face_ids.clone(),
    }
}

/// `edges` moved by `pose`.
fn moved_edges(edges: &EdgePolylines, pose: &Transform) -> EdgePolylines {
    edges
        .iter()
        .map(|line| line.iter().map(|&p| pose.point(p)).collect())
        .collect()
}

/// What a body is meshed within, beside its tolerance and angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Limit {
    /// Nothing: the mesh at the tolerance, coarsened only for a body too
    /// fine for the tessellator (`craft3d_tess::tessellate_within_budget`
    /// without a budget).
    None,
    /// A budget of the body's own: coarsened until it is within that many
    /// triangles (`tessellate_within_budget` with the budget).
    Own(usize),
    /// What a scene's budget has left: a body with more triangles is not
    /// meshed and is `TessError::OverBudget` instead
    /// (`craft3d_tess::tessellate_body_and_edges` with the budget).
    Left(usize),
}

impl Limit {
    fn allows(self, triangles: usize) -> bool {
        match self {
            Limit::None => true,
            Limit::Own(cap) | Limit::Left(cap) => triangles <= cap,
        }
    }
}

/// What a scene says of a body besides its mesh: the names of its faces,
/// edges and vertices as text, which faces each edge lies between and
/// which meet in each vertex, and its vertices. A function of the body
/// and its names, worked out once per body ([`MeshStore::facts`]) and
/// shared by the scenes that have the body.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BodyFacts {
    /// The persistent name of every face, as `document.inspect` lists
    /// them under `faceNames`. Empty when the body's faces have no names.
    pub face_names: Vec<String>,
    /// Per face, the other names it answers to (`faceAliases` of
    /// `document.inspect`). Empty when no face of the body has any.
    pub face_aliases: Vec<Vec<String>>,
    /// The persistent name of every edge, in the order of the B-rep, as
    /// `document.edges` gives it under `name`. Empty when the body's
    /// edges have no names of their own (after a boolean): an edge is
    /// then referred to by the faces it lies between.
    pub edge_names: Vec<String>,
    /// Per edge, the faces it lies between, as their places among the
    /// faces of the body, ascending: two for an edge of a solid, one for
    /// a seam. What `document.edges` lists under `faces`.
    pub edge_faces: Vec<Vec<usize>>,
    /// The body's vertices, in the order of the B-rep.
    pub vertices: Vec<Vec3>,
    /// The persistent name of every vertex, in the order of
    /// [`Self::vertices`], as `document.edges` gives it under `vertices`.
    /// Empty when the body's vertices have no names of their own (after
    /// a boolean): a vertex is then referred to by the faces that meet
    /// in it.
    pub vertex_names: Vec<String>,
    /// Per vertex, the faces that meet in it (the faces of the edges
    /// that end there), as their places among the faces of the body,
    /// ascending.
    pub vertex_faces: Vec<Vec<usize>>,
}

impl BodyFacts {
    /// The facts of `body` with `names`.
    fn of(body: &Body, names: Option<&Names>) -> Self {
        let face_names: Vec<String> = names
            .map(|names| names.faces.iter().map(ToString::to_string).collect())
            .unwrap_or_default();
        let face_aliases: Vec<Vec<String>> = names
            .filter(|names| !names.aliases.is_empty())
            .map(|names| {
                (0..names.faces.len())
                    .map(|face| {
                        names
                            .aliases_of(face)
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let edge_names: Vec<String> = names
            .map(|names| names.edge_names().map(|name| name.to_string()).collect())
            .unwrap_or_default();
        let edge_faces: Vec<Vec<usize>> = craft3d_regen::faces_of_edges(body)
            .into_iter()
            .map(|faces| faces.into_iter().map(|face| face.0 as usize).collect())
            .collect();
        let vertices: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
        let vertex_names: Vec<String> = names
            .map(|names| names.vertex_names().map(|name| name.to_string()).collect())
            .unwrap_or_default();
        let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); vertices.len()];
        for (edge, faces) in body.edges.iter().zip(&edge_faces) {
            for end in [edge.start, edge.end] {
                if let Some(list) = vertex_faces.get_mut(end.0 as usize) {
                    list.extend(
                        faces
                            .iter()
                            .filter(|face| !list.contains(face))
                            .copied()
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
        for list in &mut vertex_faces {
            list.sort_unstable();
        }
        Self {
            face_names,
            face_aliases,
            edge_names,
            edge_faces,
            vertices,
            vertex_names,
            vertex_faces,
        }
    }

    /// About how many bytes the facts hold: the names and the lists.
    fn bytes(&self) -> usize {
        let lists = |lists: &[Vec<usize>]| {
            lists
                .iter()
                .fold(0usize, |n, list| n.saturating_add(list.len()))
                .saturating_mul(std::mem::size_of::<usize>())
        };
        text_bytes(&self.face_names)
            .saturating_add(
                self.face_aliases
                    .iter()
                    .fold(0usize, |n, aliases| n.saturating_add(text_bytes(aliases))),
            )
            .saturating_add(text_bytes(&self.edge_names))
            .saturating_add(text_bytes(&self.vertex_names))
            .saturating_add(lists(&self.edge_faces))
            .saturating_add(lists(&self.vertex_faces))
            .saturating_add(
                self.vertices
                    .len()
                    .saturating_mul(std::mem::size_of::<Vec3>()),
            )
    }
}

/// How a kept mesh was sampled: the tolerance and the angle as bits, the
/// most segments a patch may have, and for a mesh that is not the plain
/// one at that sampling (coarsened for a budget of the body's own, or an
/// error) that budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MeshKey {
    tolerance: u64,
    angle: Option<u64>,
    max_segments: u32,
    own: Option<usize>,
}

/// What an entry holds of its body.
#[derive(Clone)]
enum Kept {
    Mesh(Result<BodyMesh, TessError>),
    Facts(Arc<BodyFacts>),
}

/// What an entry is keyed by besides its body.
#[derive(Clone)]
enum Key {
    Mesh(MeshKey),
    /// The facts with these names (held by a `Weak`, so that no other
    /// names can be where they were while the entry exists).
    Facts(Option<Weak<Names>>),
}

impl Key {
    fn is(&self, other: &Key) -> bool {
        match (self, other) {
            (Key::Mesh(a), Key::Mesh(b)) => a == b,
            (Key::Facts(a), Key::Facts(b)) => {
                a.as_ref().map(Weak::as_ptr) == b.as_ref().map(Weak::as_ptr)
            }
            _ => false,
        }
    }
}

struct Entry {
    body: Weak<Body>,
    key: Key,
    kept: Kept,
    bytes: usize,
    /// When it was last looked up ([`Inner::tick`]).
    used: u64,
}

struct Inner {
    entries: Vec<Entry>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

/// The meshes a session keeps, shared by every scene it builds.
#[derive(Clone)]
pub(crate) struct MeshStore(Arc<Mutex<Inner>>);

impl MeshStore {
    /// An empty store that keeps at most `budget` bytes of meshes.
    pub(crate) fn new(budget: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            entries: Vec::new(),
            tick: 0,
            bytes: 0,
            budget,
        })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps at most `bytes` of meshes from now on.
    pub(crate) fn set_budget(&self, bytes: usize) {
        let mut inner = self.lock();
        inner.budget = bytes;
        inner.evict();
    }

    /// About how many bytes of meshes are kept.
    pub(crate) fn bytes(&self) -> usize {
        self.lock().bytes
    }

    /// Lets go of what is kept for bodies that are gone (a document
    /// closed, a feature edited away): done when a scene is kept too,
    /// and here when the active document changes, so that a closed
    /// document's meshes do not wait for the next scene to be built.
    pub(crate) fn forget_gone(&self) {
        self.lock().evict();
    }

    /// How many meshes are kept.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// The mesh and the edges of `body` at `params` (its chordal tolerance
    /// and angle; `max_triangles` is `limit`'s), within `limit`: what the
    /// tessellator gives for them, found here when the body was meshed so
    /// before, made and kept otherwise.
    ///
    /// The plain mesh of a body at a tolerance and angle (made within no
    /// budget, or within one it was under) serves every limit it is
    /// within: a mesh within a budget is the very mesh made without one
    /// (the tessellator's promise, `TessParams::max_triangles`). A mesh
    /// coarsened for a budget of the body's own, and an error, serve only
    /// the limit they were made under; a scene's budget ([`Limit::Left`])
    /// keeps only a mesh that was within it.
    ///
    /// A placed body ([`note_placed`]) is the mesh of the body it was
    /// placed from, moved.
    pub(crate) fn mesh(
        &self,
        body: &Arc<Body>,
        params: &TessParams,
        limit: Limit,
    ) -> Result<BodyMesh, TessError> {
        let Some((source, pose)) = root_of(body) else {
            return self.mesh_own(body, params, limit);
        };
        let made = self.mesh_own(&source, params, limit)?;
        Ok(BodyMesh {
            mesh: Arc::new(moved_mesh(&made.mesh, &pose)),
            edges: Arc::new(moved_edges(&made.edges, &pose)),
            coarsened: made.coarsened,
            placed: Some((made.mesh, pose)),
        })
    }

    /// [`Self::mesh`] of the body itself.
    fn mesh_own(
        &self,
        body: &Arc<Body>,
        params: &TessParams,
        limit: Limit,
    ) -> Result<BodyMesh, TessError> {
        let plain = MeshKey {
            tolerance: params.chordal_tolerance.to_bits(),
            angle: params.max_angle.map(f64::to_bits),
            max_segments: params.max_segments,
            own: None,
        };
        let exact = match limit {
            Limit::None => Some(plain),
            Limit::Own(cap) => Some(MeshKey {
                own: Some(cap),
                ..plain
            }),
            Limit::Left(_) => None,
        };
        {
            let mut inner = self.lock();
            if let Some(key) = exact
                && let Some(Kept::Mesh(found)) = inner.find(body, &Key::Mesh(key))
            {
                return found;
            }
            if exact != Some(plain)
                && let Some(Kept::Mesh(Ok(found))) = inner.find(body, &Key::Mesh(plain))
                && !found.coarsened
                && limit.allows(found.mesh.triangles.len())
            {
                return Ok(found);
            }
        }
        // Outside the lock: the work the store saves.
        let made = match limit {
            Limit::None | Limit::Own(_) => {
                let cap = match limit {
                    Limit::Own(cap) => Some(cap),
                    _ => None,
                };
                tessellate_within_budget(
                    body,
                    &TessParams {
                        max_triangles: cap,
                        ..*params
                    },
                )
                .map(|within| BodyMesh {
                    mesh: Arc::new(within.mesh),
                    edges: Arc::new(within.edges),
                    coarsened: within.tolerance > params.chordal_tolerance,
                    placed: None,
                })
            }
            Limit::Left(left) => tessellate_body_and_edges(
                body,
                &TessParams {
                    max_triangles: Some(left),
                    ..*params
                },
            )
            .and_then(|both| {
                both.mesh.map(|mesh| BodyMesh {
                    mesh: Arc::new(mesh),
                    edges: Arc::new(both.edges),
                    coarsened: false,
                    placed: None,
                })
            }),
        };
        if let Some(key) = key_to_keep(&made, limit, plain, exact) {
            let bytes = entry_bytes(made.as_ref().map_or(0, body_mesh_bytes), &made);
            self.lock()
                .keep(body, Key::Mesh(key), Kept::Mesh(made.clone()), bytes);
        }
        made
    }

    /// The facts of `body` with `names` ([`BodyFacts`]): found here when
    /// they were worked out before, worked out and kept otherwise.
    /// A placed body's are those of the body it was placed from, its
    /// vertices moved.
    pub(crate) fn facts(&self, body: &Arc<Body>, names: Option<&Arc<Names>>) -> Arc<BodyFacts> {
        if let Some((source, pose)) = root_of(body)
            && source.vertices.len() == body.vertices.len()
            && source.edges.len() == body.edges.len()
            && source.faces.len() == body.faces.len()
        {
            let of_source = self.facts_own(&source, names);
            let mut facts = (*of_source).clone();
            for v in &mut facts.vertices {
                *v = pose.point(*v);
            }
            return Arc::new(facts);
        }
        self.facts_own(body, names)
    }

    /// [`Self::facts`] of the body itself.
    fn facts_own(&self, body: &Arc<Body>, names: Option<&Arc<Names>>) -> Arc<BodyFacts> {
        let key = Key::Facts(names.map(Arc::downgrade));
        if let Some(Kept::Facts(found)) = self.lock().find(body, &key) {
            return found;
        }
        let facts = Arc::new(BodyFacts::of(body, names.map(Arc::as_ref)));
        let bytes = facts.bytes().saturating_add(std::mem::size_of::<Entry>());
        self.lock()
            .keep(body, key, Kept::Facts(Arc::clone(&facts)), bytes);
        facts
    }
}

/// Under which key what the tessellator made for `limit` is kept, or
/// `None` for what is not kept: `plain` is the key of the mesh at the
/// sampling without a budget, `exact` the key of the limit itself when
/// it has one (none for what a scene's budget has left).
fn key_to_keep(
    made: &Result<BodyMesh, TessError>,
    limit: Limit,
    plain: MeshKey,
    exact: Option<MeshKey>,
) -> Option<MeshKey> {
    match (made, exact) {
        // The plain mesh at the tolerance: within the limit it was made
        // under, so the mesh made without one.
        (Ok(meshed), _) if !meshed.coarsened && limit.allows(meshed.mesh.triangles.len()) => {
            Some(plain)
        }
        // The work allowance of the operation that asked ran out
        // (`craft3d_geom::allowance`): a state of the caller, not of the
        // body; asked again with the allowance of another operation, the
        // body may well mesh.
        (Err(TessError::Spent), _) => None,
        // Coarsened for a budget of its own, or failed under it (or under
        // none): what that limit gives, and only it.
        (_, Some(exact)) => Some(exact),
        // Over what the scene had left by a handful (faces parted after
        // the budget was looked at), or the budget's error: the scene
        // discards it, and so does the store.
        (_, None) => None,
    }
}

/// What an entry of `payload` bytes of meshes or facts weighs in the
/// budget: the entry itself and, for an error, its text too, so that an
/// error weighs something and a body that is refused at many samplings
/// cannot fill the store with entries that are never let go.
fn entry_bytes<T>(payload: usize, made: &Result<T, TessError>) -> usize {
    let own = std::mem::size_of::<Entry>();
    let text = made.as_ref().err().map_or(0, |e| e.to_string().len());
    payload.saturating_add(own).saturating_add(text)
}

impl Inner {
    /// What is kept for `body` under `key`, marked as used now.
    fn find(&mut self, body: &Arc<Body>, key: &Key) -> Option<Kept> {
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        let entry = self
            .entries
            .iter_mut()
            // A `Weak` keeps its allocation, so no other body can be
            // where a kept one was.
            .find(|entry| {
                std::ptr::eq(entry.body.as_ptr(), Arc::as_ptr(body)) && entry.key.is(key)
            })?;
        entry.used = tick;
        Some(entry.kept.clone())
    }

    /// Keeps `kept` for `body` under `key` (unless it is there already:
    /// another thread made the same thing meanwhile), then lets bodies
    /// that are gone and the least recently used go until within the
    /// budget.
    fn keep(&mut self, body: &Arc<Body>, key: Key, kept: Kept, bytes: usize) {
        if self
            .entries
            .iter()
            .any(|entry| std::ptr::eq(entry.body.as_ptr(), Arc::as_ptr(body)) && entry.key.is(&key))
        {
            return;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.push(Entry {
            body: Arc::downgrade(body),
            key,
            kept,
            bytes,
            used: self.tick,
        });
        self.evict();
    }

    fn evict(&mut self) {
        // A body no regeneration holds any more cannot be asked for again.
        let mut bytes = self.bytes;
        self.entries.retain(|entry| {
            let alive = entry.body.strong_count() > 0;
            if !alive {
                bytes = bytes.saturating_sub(entry.bytes);
            }
            alive
        });
        self.bytes = bytes;
        while self.bytes > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(at, _)| at)
            else {
                break;
            };
            let gone = self.entries.remove(oldest);
            self.bytes = self.bytes.saturating_sub(gone.bytes);
        }
    }
}

/// About how many bytes a mesh holds: its points, normals, triangles and
/// face ids.
pub(crate) fn mesh_bytes(mesh: &Mesh) -> usize {
    let point = std::mem::size_of::<Vec3>();
    mesh.positions
        .len()
        .saturating_add(mesh.normals.len())
        .saturating_mul(point)
        .saturating_add(mesh.triangles.len().saturating_mul(12))
        .saturating_add(mesh.face_ids.len().saturating_mul(4))
}

/// About how many bytes polylines hold: their points.
pub(crate) fn points_bytes(lines: &[Vec<Vec3>]) -> usize {
    lines
        .iter()
        .fold(0usize, |n, line| n.saturating_add(line.len()))
        .saturating_mul(std::mem::size_of::<Vec3>())
}

/// About how many bytes names hold: their text and their headers.
pub(crate) fn text_bytes(names: &[String]) -> usize {
    names.iter().fold(0usize, |n, name| {
        n.saturating_add(name.len()).saturating_add(24)
    })
}

fn body_mesh_bytes(meshed: &BodyMesh) -> usize {
    mesh_bytes(&meshed.mesh).saturating_add(points_bytes(&meshed.edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_kernel::primitives;

    fn frame() -> craft3d_math::Frame {
        craft3d_math::Frame::from_z(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)).expect("a frame")
    }

    fn block(size: f64) -> Arc<Body> {
        Arc::new(primitives::block(&frame(), size, size, size).expect("a block"))
    }

    fn params(tolerance: f64) -> TessParams {
        TessParams {
            chordal_tolerance: tolerance,
            ..TessParams::default()
        }
    }

    /// A body placed from a body placed from a source (a subassembly's
    /// instance inside an assembly) is the source's mesh moved by the
    /// inner pose first and the outer one after it, with the source
    /// meshed once; a mirror is not noted, and that body is meshed itself.
    #[test]
    fn a_nested_placement_composes_its_poses_and_meshes_the_source_once() {
        let store = MeshStore::new(usize::MAX);
        let source = block(2.0);
        let inner = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(5.0, -1.0, 2.0)));
        let outer = Transform::rotation(Vec3::new(1.0, 1.0, 0.0), -0.4)
            .unwrap()
            .then(&Transform::translation(Vec3::new(-3.0, 4.0, 0.5)));
        let middle = Arc::new(source.transformed(&inner).unwrap());
        note_placed(&middle, &source, &inner);
        let placed = Arc::new(middle.transformed(&outer).unwrap());
        note_placed(&placed, &middle, &outer);
        let (root, pose) = root_of(&placed).expect("placed");
        assert!(Arc::ptr_eq(&root, &source));
        assert!(Arc::ptr_eq(&measured_as(&placed), &source));
        let own = store.mesh(&source, &params(0.1), Limit::None).unwrap();
        let moved = store.mesh(&placed, &params(0.1), Limit::None).unwrap();
        // Only the source was meshed and kept.
        assert_eq!(store.len(), 1);
        let (from, by) = moved.placed.clone().expect("a placed mesh");
        assert!(Arc::ptr_eq(&from, &own.mesh));
        assert_eq!(by, pose);
        for (p, q) in moved.mesh.positions.iter().zip(&own.mesh.positions) {
            // Inner first, then outer, within rounding.
            let twice = outer.point(inner.point(*q));
            assert!((*p - twice).length() < 1e-12, "{p:?} {twice:?}");
            // And bit for bit the composed pose.
            assert_eq!(*p, pose.point(*q));
        }
        // Its vertices are the placed body's own, within rounding.
        let facts = store.facts(&placed, None);
        for (v, w) in facts.vertices.iter().zip(&placed.vertices) {
            assert!((*v - w.point).length() < 1e-12);
        }
        // A mirror is not noted: the body is meshed itself.
        let mirror = Transform::mirror(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap();
        let mirrored = Arc::new(source.transformed(&mirror).unwrap());
        note_placed(&mirrored, &source, &mirror);
        assert!(root_of(&mirrored).is_none());
        let own_mirror = store.mesh(&mirrored, &params(0.1), Limit::None).unwrap();
        assert!(own_mirror.placed.is_none());
    }

    /// The same body at the same sampling is meshed once; another
    /// tolerance, another angle or another body is meshed again.
    #[test]
    fn a_body_is_meshed_once_per_sampling() {
        let store = MeshStore::new(usize::MAX);
        let body = block(2.0);
        let first = store.mesh(&body, &params(0.1), Limit::None).unwrap();
        let again = store.mesh(&body, &params(0.1), Limit::None).unwrap();
        assert!(Arc::ptr_eq(&first.mesh, &again.mesh));
        assert!(Arc::ptr_eq(&first.edges, &again.edges));
        assert_eq!(store.len(), 1);
        store.mesh(&body, &params(0.2), Limit::None).unwrap();
        store
            .mesh(
                &body,
                &TessParams {
                    max_angle: Some(0.2),
                    ..params(0.1)
                },
                Limit::None,
            )
            .unwrap();
        let other = block(3.0);
        store.mesh(&other, &params(0.1), Limit::None).unwrap();
        assert_eq!(store.len(), 4);
        // A copy of the body with the same geometry is another body.
        let copy = Arc::new((*body).clone());
        store.mesh(&copy, &params(0.1), Limit::None).unwrap();
        assert_eq!(store.len(), 5);
    }

    /// The plain mesh serves a budget it is within, and a budget the mesh
    /// is within gives the plain mesh; a budget it is over is meshed
    /// afresh (a block has 12 triangles).
    #[test]
    fn the_plain_mesh_serves_every_limit_it_is_within() {
        let store = MeshStore::new(usize::MAX);
        let body = block(2.0);
        let plain = store.mesh(&body, &params(0.1), Limit::None).unwrap();
        assert_eq!(plain.mesh.triangles.len(), 12);
        for limit in [
            Limit::Own(12),
            Limit::Left(12),
            Limit::Own(1000),
            Limit::Left(1000),
        ] {
            let within = store.mesh(&body, &params(0.1), limit).unwrap();
            assert!(Arc::ptr_eq(&plain.mesh, &within.mesh), "{limit:?}");
            assert!(!within.coarsened);
        }
        assert_eq!(store.len(), 1);
        // Over what a scene has left: an error, not kept.
        let over = store.mesh(&body, &params(0.1), Limit::Left(11));
        assert!(
            matches!(over, Err(TessError::OverBudget { .. })),
            "{over:?}"
        );
        assert_eq!(store.len(), 1);

        // The other way round: made under a budget first, then without.
        let store = MeshStore::new(usize::MAX);
        let under = store.mesh(&body, &params(0.1), Limit::Left(100)).unwrap();
        let without = store.mesh(&body, &params(0.1), Limit::None).unwrap();
        assert!(Arc::ptr_eq(&under.mesh, &without.mesh));
        let own = store.mesh(&body, &params(0.1), Limit::Own(50)).unwrap();
        assert!(Arc::ptr_eq(&own.mesh, &without.mesh));
        assert_eq!(store.len(), 1);

        // A curved body, many triangles: what each limit keeps is what
        // a store that knows nothing makes under the same limit, bit for
        // bit, and what it made without one.
        let body = Arc::new(primitives::cylinder(&frame(), 1.0, 2.0).expect("a cylinder"));
        let fresh = |limit: Limit| {
            MeshStore::new(usize::MAX)
                .mesh(&body, &params(1e-3), limit)
                .unwrap()
        };
        let plain = fresh(Limit::None);
        let count = plain.mesh.triangles.len();
        assert!(count > 100);
        for limit in [
            Limit::Left(count),
            Limit::Own(count),
            Limit::Left(usize::MAX),
        ] {
            assert_eq!(*fresh(limit).mesh, *plain.mesh, "{limit:?}");
            assert_eq!(*fresh(limit).edges, *plain.edges, "{limit:?}");
        }
        let store = MeshStore::new(usize::MAX);
        let under = store
            .mesh(&body, &params(1e-3), Limit::Left(count))
            .unwrap();
        let without = store.mesh(&body, &params(1e-3), Limit::None).unwrap();
        assert!(Arc::ptr_eq(&under.mesh, &without.mesh));
        assert_eq!(*without.mesh, *plain.mesh);
    }

    /// A mesh coarsened for a budget of its own is kept for that budget
    /// only; the plain mesh is still made when asked for.
    #[test]
    fn a_coarsened_mesh_serves_its_own_budget_only() {
        let store = MeshStore::new(usize::MAX);
        let body = Arc::new(primitives::cylinder(&frame(), 1.0, 2.0).expect("a cylinder"));
        let fine = store.mesh(&body, &params(1e-4), Limit::None).unwrap();
        assert!(!fine.coarsened && fine.mesh.triangles.len() > 100);
        let coarse = store.mesh(&body, &params(1e-4), Limit::Own(100)).unwrap();
        assert!(coarse.coarsened && coarse.mesh.triangles.len() <= 100);
        assert_eq!(store.len(), 2);
        let coarse_again = store.mesh(&body, &params(1e-4), Limit::Own(100)).unwrap();
        assert!(Arc::ptr_eq(&coarse.mesh, &coarse_again.mesh));
        let fine_again = store.mesh(&body, &params(1e-4), Limit::None).unwrap();
        assert!(Arc::ptr_eq(&fine.mesh, &fine_again.mesh));
        // Another own budget the coarse mesh happens to be within is not
        // served by it: it is the ladder's result for its own budget.
        let other = store.mesh(&body, &params(1e-4), Limit::Own(200)).unwrap();
        assert!(!Arc::ptr_eq(&coarse.mesh, &other.mesh));
        assert_eq!(store.len(), 3);
    }

    /// The facts of a body are worked out once per body and names, and
    /// are what working them out gives.
    #[test]
    fn the_facts_of_a_body_are_worked_out_once() {
        let store = MeshStore::new(usize::MAX);
        let body = block(2.0);
        let names = Arc::new(craft3d_regen::Names::numbered(
            craft3d_doc::FeatureId(7),
            &body,
        ));
        let first = store.facts(&body, Some(&names));
        assert_eq!(*first, BodyFacts::of(&body, Some(&names)));
        assert_eq!(first.face_names.len(), 6);
        assert_eq!(first.edge_faces.len(), 12);
        assert!(first.edge_faces.iter().all(|faces| faces.len() == 2));
        assert_eq!(first.vertices.len(), 8);
        assert!(first.vertex_faces.iter().all(|faces| faces.len() == 3));
        let again = store.facts(&body, Some(&names));
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(store.len(), 1);
        // Without names, or with other names: other facts.
        let plain = store.facts(&body, None);
        assert!(plain.face_names.is_empty() && plain.edge_faces.len() == 12);
        let other = Arc::new(craft3d_regen::Names::numbered(
            craft3d_doc::FeatureId(8),
            &body,
        ));
        assert!(!Arc::ptr_eq(&store.facts(&body, Some(&other)), &first));
        assert_eq!(store.len(), 3);
        assert!(store.bytes() > 0);
    }

    /// An error is remembered for the limit it was made under, and
    /// weighs something: a body that is refused at many samplings is
    /// held to the budget like meshes are.
    #[test]
    fn an_error_is_remembered_for_its_limit_and_weighs_something() {
        let store = MeshStore::new(usize::MAX);
        let body = block(2.0);
        let invalid = params(f64::NAN);
        assert!(store.mesh(&body, &invalid, Limit::None).is_err());
        assert!(store.mesh(&body, &invalid, Limit::None).is_err());
        assert_eq!(store.len(), 1);
        let each = store.bytes();
        assert!(each >= std::mem::size_of::<Entry>(), "{each} bytes");

        // A body with a face that cannot be meshed (its loop names an
        // edge that is not there), asked for at a hundred tolerances:
        // within a budget of ten entries' worth.
        let mut broken = (*block(3.0)).clone();
        broken.loops[0].coedges[0].edge = craft3d_topo::EdgeId(99);
        let broken = Arc::new(broken);
        let store = MeshStore::new(each * 10);
        for k in 1..=100 {
            let refused = store.mesh(&broken, &params(1e-3 * k as f64), Limit::None);
            assert!(
                matches!(refused, Err(TessError::InvalidBody(_))),
                "{refused:?}"
            );
        }
        assert!(store.len() <= 10, "{} entries", store.len());
        assert!(store.bytes() <= each * 10);
    }

    /// The limit on segments is part of the sampling: another limit is
    /// another mesh.
    #[test]
    fn the_segment_limit_is_part_of_the_sampling() {
        let store = MeshStore::new(usize::MAX);
        let body = Arc::new(primitives::cylinder(&frame(), 1.0, 2.0).expect("a cylinder"));
        let fine = params(1e-5);
        let meshed = store.mesh(&body, &fine, Limit::None).unwrap();
        let few = TessParams {
            max_segments: 8,
            ..fine
        };
        let coarse = store.mesh(&body, &few, Limit::None).unwrap();
        assert_eq!(store.len(), 2);
        assert!(!Arc::ptr_eq(&meshed.mesh, &coarse.mesh));
        assert!(coarse.coarsened || coarse.mesh.triangles.len() < meshed.mesh.triangles.len());
        let again = store.mesh(&body, &few, Limit::None).unwrap();
        assert!(Arc::ptr_eq(&coarse.mesh, &again.mesh));
    }

    /// A body the tessellator could not finish within the work allowance
    /// of the operation that asked (`TessError::Spent`) is not remembered
    /// as refused: with another allowance it meshes.
    #[test]
    fn running_out_of_the_work_allowance_is_not_remembered() {
        let store = MeshStore::new(usize::MAX);
        let body = Arc::new(primitives::cylinder(&frame(), 1.0, 2.0).expect("a cylinder"));
        let spent =
            craft3d_geom::allowance::within(1, || store.mesh(&body, &params(1e-4), Limit::None));
        assert!(matches!(spent, Err(TessError::Spent)), "{spent:?}");
        assert_eq!(store.len(), 0);
        assert!(store.mesh(&body, &params(1e-4), Limit::None).is_ok());
        assert_eq!(store.len(), 1);
        // The rule itself, for an allowance that runs out under any limit.
        let plain = MeshKey {
            tolerance: 1.0f64.to_bits(),
            angle: None,
            max_segments: 1024,
            own: None,
        };
        for limit in [Limit::None, Limit::Own(10), Limit::Left(10)] {
            let exact = match limit {
                Limit::None => Some(plain),
                Limit::Own(cap) => Some(MeshKey {
                    own: Some(cap),
                    ..plain
                }),
                Limit::Left(_) => None,
            };
            assert_eq!(
                key_to_keep(&Err(TessError::Spent), limit, plain, exact),
                None
            );
            assert_eq!(
                key_to_keep(&Err(TessError::TooManyVertices), limit, plain, exact),
                exact
            );
        }
    }

    /// Within the budget the least recently used meshes go first, and the
    /// meshes of bodies that are gone go at once.
    #[test]
    fn the_budget_lets_the_least_recently_used_go() {
        let store = MeshStore::new(usize::MAX);
        let bodies: Vec<Arc<Body>> = (1..=4).map(|k| block(k as f64)).collect();
        let first: Vec<BodyMesh> = bodies
            .iter()
            .map(|body| store.mesh(body, &params(0.1), Limit::None).unwrap())
            .collect();
        let each = store.bytes() / 4;
        assert!(each > 0);
        // Room for two: the two used last stay.
        store.mesh(&bodies[0], &params(0.1), Limit::None).unwrap();
        store.mesh(&bodies[2], &params(0.1), Limit::None).unwrap();
        store.set_budget(each * 2);
        assert_eq!(store.len(), 2);
        for k in [0, 2] {
            let again = store.mesh(&bodies[k], &params(0.1), Limit::None).unwrap();
            assert!(
                Arc::ptr_eq(&first[k].mesh, &again.mesh),
                "block {k} was let go"
            );
        }
        // A body that is gone takes its mesh with it.
        let store = MeshStore::new(usize::MAX);
        let gone = block(5.0);
        store.mesh(&gone, &params(0.1), Limit::None).unwrap();
        assert_eq!(store.len(), 1);
        drop(gone);
        store.mesh(&bodies[1], &params(0.1), Limit::None).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.bytes(), each);
    }
}
