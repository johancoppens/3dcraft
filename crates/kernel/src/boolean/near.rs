//! Near approaches the section does not see (2026-11-24).
//!
//! The section refuses two points it registers (the operands' vertices and
//! the cuts it makes) between half and twice the tolerance apart, and
//! [`super::zero_thickness`] looks for parts that touch at the places where
//! the operands meet. Neither looks at the inside of an edge or of a face
//! against the other body where no surfaces meet nearby: two edges that
//! cross 1.9 tolerances apart, an edge that passes a face, two rods side by
//! side, a ball over a plane, a vertex beside an edge. This finds them: in a
//! boolean's result ([`in_result`]), or between the two operands
//! ([`between`], for `contact`), the pairs of a vertex, an edge or a face
//! and another that share no vertex and come less than twice the tolerance
//! apart, of which one lies on a face of A and the other on a face of B.
//!
//! **What is looked at.** Only pairs that can be such an approach:
//!
//! - The section met every pair of faces (one of A, one of B) that it did
//!   not prove further apart than twice the tolerance (boxes, hulls, covers:
//!   `section::to_meet`). An entity lies on the faces around it, and a face
//!   of the result on the faces of the operands it is a piece of, so two
//!   entities come that near only where a face of A around one and a face
//!   of B around the other were met, or where both lie on faces of one
//!   operand: an approach that operand had already, not looked at. This is
//!   read from the sources before any geometry.
//! - Their boxes, grown by twice the tolerance, meet: a tree of the boxes of
//!   one side, asked with each box of the other.
//! - They share no vertex: joined there (a face and its own edge, a wedge).
//! - Not two vertices: every vertex of a result is a point the section
//!   registered, and two of those in the band are refused there.
//!
//! **How near.** A vertex to an edge or a face by its nearest point; an edge
//! to an edge or a face by its distance sampled along the edge, each stretch
//! between samples bounded below by the edge's speed, and only the stretches
//! that may come within twice the tolerance searched closer (golden
//! sections); two faces by alternating projections from three seeds. A
//! straight edge over a plane and two planes come nearest at a boundary,
//! which is looked at as its own edges and vertices. Before any of that,
//! a pair is passed over where the other surface is far from the box of an
//! analytic entity, or where the covers of the operands' faces prove a
//! freeform face that far. An approach counts only where two points of the
//! two entities, each evaluated exactly, are less than twice the tolerance
//! apart, inside both: a search that misses misses, it never finds an
//! approach that is not there.
//!
//! **What is done with one** ([`super::near_approaches`], and `contact`'s
//! evidence): two parts that come that near touch (zero thickness, the
//! developer's rule of 2026-10-15); a sliver beside a vertex is left to the
//! wedge readings (2026-10-18); two walls of one shell between half and
//! twice the tolerance apart are refused as too near to tell apart
//! (2026-10-22). Measured on the benchmark's models on 2026-11-24: about a
//! hundredth of the booleans' own time (docs/design/booleans.md › *Near
//! approaches inside an edge or a face*).

use super::cover::Cover;
use super::facetree::FaceTree;
use super::freeform::is_freeform;
use super::hits::speed_bound;
use super::outline::{self, Outline, Sheet, Side, boxes_meet};
use super::section::{self, Solid};
use super::{BooleanResult, Operand};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Vec3};
use craft3d_topo::{Body, Edge, EdgeId, FaceId};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// Samples along an edge.
const SAMPLES: usize = 24;
/// Stretches of an edge searched closer, at most, per pair.
const SEARCHES: usize = 3;
/// Steps of a golden-section search.
const GOLDEN: usize = 32;
/// Steps of the alternating projection between two faces.
const ALTERNATE: usize = 40;
/// A parameter this close to an end of its range (as a share of the
/// range) is at that end: the vertex, looked at as one.
const END: f64 = 1e-7;
/// Pairs measured in one look, at most: past that the rest are not looked
/// at, as before this look existed (a miss, never a refusal).
const MOST_PAIRS: usize = 1 << 16;
/// One over the golden ratio.
const INVERSE_GOLDEN: f64 = 0.618_033_988_749_894_9;

/// An entity of a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Ent {
    Vertex(u32),
    Edge(u32),
    Face(u32),
}

/// An entity and the body it is in: 0, the result or A; 1, B.
pub(super) type Who = (usize, Ent);

/// Two entities that share no vertex and come less than twice the
/// tolerance apart.
#[derive(Clone, Copy, Debug)]
pub(super) struct Approach {
    pub x: Who,
    pub y: Who,
    /// The nearest points found, on `x` and on `y`.
    pub p: Vec3,
    pub q: Vec3,
    /// Their distance.
    pub distance: f64,
    /// In different shells of the result: two parts.
    pub parts: bool,
    /// A vertex of one and a vertex of the other joined by an edge: a
    /// sliver or a wedge beside a vertex.
    pub hop: bool,
    /// A face of the operands at each, for naming the pair: one of A and
    /// one of B where there are.
    pub faces: [(Operand, FaceId); 2],
}

/// The approaches in a boolean's result (one body): pairs of its entities
/// of which one lies on a face of A and the other on a face of B (see the
/// module), with `met` the pairs of faces the section met. In a fixed
/// order.
pub(super) fn in_result<'r, 'x>(
    result: &'r BooleanResult,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    met: &[(usize, usize)],
    tol: f64,
) -> Vec<Approach> {
    look(Bodies::Result(result), vec![&result.body], a, b, met, tol)
}

/// The approaches between the two operands themselves: pairs of an entity
/// of A (body 0) and one of B (body 1).
pub(super) fn between<'r, 'x>(
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    met: &[(usize, usize)],
    tol: f64,
) -> Vec<Approach> {
    look(Bodies::Operands, vec![a.body, b.body], a, b, met, tol)
}

/// Where the bodies looked at come from.
#[derive(Clone, Copy)]
enum Bodies<'r> {
    /// One body, a boolean's result: each face a piece of faces of the
    /// operands (`merged`).
    Result(&'r BooleanResult),
    /// The operands: body 0 is A, body 1 is B.
    Operands,
}

/// Lists of values by key: per key, its values ascending, each once.
struct Lists {
    start: Vec<usize>,
    values: Vec<u32>,
}

impl Lists {
    fn new(keys: usize, mut pairs: Vec<(u32, u32)>) -> Lists {
        pairs.retain(|&(k, _)| (k as usize) < keys);
        pairs.sort_unstable();
        pairs.dedup();
        let mut start = vec![0usize; keys.saturating_add(1)];
        for &(k, _) in &pairs {
            if let Some(slot) = start.get_mut((k as usize).saturating_add(1)) {
                *slot += 1;
            }
        }
        let mut total = 0usize;
        for slot in &mut start {
            total += *slot;
            *slot = total;
        }
        Lists {
            start,
            values: pairs.into_iter().map(|(_, v)| v).collect(),
        }
    }

    fn of(&self, key: u32) -> &[u32] {
        let k = key as usize;
        match (self.start.get(k), self.start.get(k.saturating_add(1))) {
            (Some(&from), Some(&to)) if from <= to => self.values.get(from..to).unwrap_or(&[]),
            _ => &[],
        }
    }
}

fn index(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// A body's topology as the look reads it.
struct Topo<'r> {
    body: &'r Body,
    /// Per edge, the faces it bounds.
    edge_faces: Lists,
    /// Per vertex, the edges at it.
    vertex_edges: Lists,
    /// Per face, the shell it is in.
    face_shell: Vec<usize>,
    /// Per face, its vertices, made when first asked for.
    face_vertices: Vec<Option<Vec<u32>>>,
    /// Per face, its outline, made when first asked for.
    outlines: Vec<Option<Option<Outline>>>,
    /// Per face, the box of what it holds (`Body::held_face_bbox`), made
    /// when first asked for: for a piece of a ball or a ring much smaller
    /// than the box of the whole, which the pairing takes (dearer to make,
    /// so made only for the faces a pair of faces is measured on).
    held: Vec<Option<Option<Bbox3>>>,
}

impl<'r> Topo<'r> {
    fn new(body: &'r Body) -> Topo<'r> {
        let mut pairs = Vec::new();
        for (f, face) in body.faces.iter().enumerate() {
            for l in &face.loops {
                if let Some(lp) = body.loop_(*l) {
                    pairs.extend(lp.coedges.iter().map(|c| (c.edge.0, index(f))));
                }
            }
        }
        let edge_faces = Lists::new(body.edges.len(), pairs);
        let mut pairs = Vec::with_capacity(2 * body.edges.len());
        for (e, edge) in body.edges.iter().enumerate() {
            pairs.push((edge.start.0, index(e)));
            pairs.push((edge.end.0, index(e)));
        }
        let vertex_edges = Lists::new(body.vertices.len(), pairs);
        let mut face_shell = vec![usize::MAX; body.faces.len()];
        for (s, shell) in body.shells.iter().enumerate() {
            for f in &shell.faces {
                if let Some(slot) = face_shell.get_mut(f.0 as usize) {
                    *slot = s;
                }
            }
        }
        Topo {
            body,
            edge_faces,
            vertex_edges,
            face_shell,
            face_vertices: vec![None; body.faces.len()],
            outlines: (0..body.faces.len()).map(|_| None).collect(),
            held: vec![None; body.faces.len()],
        }
    }

    /// The faces around an entity, ascending, each once.
    fn faces(&self, ent: Ent) -> Vec<u32> {
        match ent {
            Ent::Face(f) => vec![f],
            Ent::Edge(e) => self.edge_faces.of(e).to_vec(),
            Ent::Vertex(v) => {
                let mut out: Vec<u32> = self
                    .vertex_edges
                    .of(v)
                    .iter()
                    .flat_map(|&e| self.edge_faces.of(e).iter().copied())
                    .collect();
                out.sort_unstable();
                out.dedup();
                out
            }
        }
    }

    /// Makes the list of face `f`'s vertices if it is not made yet.
    fn ensure_vertices(&mut self, f: u32) {
        let body = self.body;
        let Some(slot) = self.face_vertices.get_mut(f as usize) else {
            return;
        };
        if slot.is_some() {
            return;
        }
        let mut out = Vec::new();
        if let Some(face) = body.faces.get(f as usize) {
            for l in &face.loops {
                let Some(lp) = body.loop_(*l) else {
                    continue;
                };
                for c in &lp.coedges {
                    if let Some(edge) = body.edge(c.edge) {
                        out.push(edge.start.0);
                        out.push(edge.end.0);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        *slot = Some(out);
    }

    /// The vertices of an entity, ascending, each once (of a face, once
    /// [`Topo::ensure_vertices`] made them).
    fn vertices(&self, ent: Ent) -> Cow<'_, [u32]> {
        match ent {
            Ent::Vertex(v) => Cow::Owned(vec![v]),
            Ent::Edge(e) => match self.body.edges.get(e as usize) {
                Some(edge) => {
                    let mut v = vec![edge.start.0, edge.end.0];
                    v.sort_unstable();
                    v.dedup();
                    Cow::Owned(v)
                }
                None => Cow::Owned(Vec::new()),
            },
            Ent::Face(f) => Cow::Borrowed(
                self.face_vertices
                    .get(f as usize)
                    .and_then(|list| list.as_deref())
                    .unwrap_or(&[]),
            ),
        }
    }

    /// Whether two entities share a vertex: the shorter list of vertices
    /// looked up in the longer.
    fn share_vertex(&mut self, x: Ent, y: Ent) -> bool {
        for ent in [x, y] {
            if let Ent::Face(f) = ent {
                self.ensure_vertices(f);
            }
        }
        let (vx, vy) = (self.vertices(x), self.vertices(y));
        let (few, many) = if vx.len() <= vy.len() {
            (&vx, &vy)
        } else {
            (&vy, &vx)
        };
        few.iter().any(|v| many.binary_search(v).is_ok())
    }

    /// The shell an entity is in (`usize::MAX` where none is known).
    fn shell(&self, ent: Ent) -> usize {
        self.faces(ent)
            .first()
            .and_then(|&f| self.face_shell.get(f as usize))
            .copied()
            .unwrap_or(usize::MAX)
    }

    /// Makes the outline of face `f` if it is not made yet; whether it has
    /// one.
    fn ensure(&mut self, f: u32, tol: f64) -> bool {
        let body = self.body;
        let Some(slot) = self.outlines.get_mut(f as usize) else {
            return false;
        };
        if slot.is_none() {
            *slot = Some(section::face_outline(body, f as usize, tol).ok());
        }
        matches!(slot, Some(Some(_)))
    }

    /// The box of what face `f` holds ([`Topo::held`]).
    fn held_box(&mut self, f: u32) -> Option<Bbox3> {
        let body = self.body;
        let slot = self.held.get_mut(f as usize)?;
        *slot.get_or_insert_with(|| body.held_face_bbox(FaceId(f)).ok())
    }

    /// The outline of face `f` when [`Topo::ensure`] made it.
    fn made(&self, f: u32) -> Option<&Outline> {
        self.outlines.get(f as usize)?.as_ref()?.as_ref()
    }
}

/// An entity with its box, for the pairing.
struct Item {
    who: Who,
    bbox: Bbox3,
}

/// What one look knows.
struct Look<'r, 'x> {
    whence: Bodies<'r>,
    topos: Vec<Topo<'r>>,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    /// The pairs of faces (of A, of B) the section met.
    met: BTreeSet<(u32, u32)>,
    tol: f64,
    near: f64,
}

fn look<'r, 'x>(
    whence: Bodies<'r>,
    bodies: Vec<&'r Body>,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    met: &[(usize, usize)],
    tol: f64,
) -> Vec<Approach> {
    let near = 2.0 * tol;
    if met.is_empty() || !(near.is_finite() && near > 0.0) {
        return Vec::new();
    }
    let met: BTreeSet<(u32, u32)> = met.iter().map(|&(fa, fb)| (index(fa), index(fb))).collect();
    let mut look = Look {
        whence,
        topos: bodies.into_iter().map(Topo::new).collect(),
        a,
        b,
        met,
        tol,
        near,
    };
    let pairs = look.pairs();
    look.measure(pairs)
}

/// The rank of an entity's kind: vertices, edges, faces.
fn rank(ent: Ent) -> u8 {
    match ent {
        Ent::Vertex(_) => 0,
        Ent::Edge(_) => 1,
        Ent::Face(_) => 2,
    }
}

impl<'r, 'x> Look<'r, 'x> {
    fn solid(&self, operand: Operand) -> &'r Solid<'x> {
        match operand {
            Operand::A => self.a,
            Operand::B => self.b,
        }
    }

    /// The faces of the operands that face `f` of body `body` is a piece
    /// of (itself, for an operand).
    fn sources(&self, body: usize, f: u32) -> Vec<(Operand, u32)> {
        match self.whence {
            Bodies::Result(result) => {
                match result.merged.get(f as usize).filter(|m| !m.is_empty()) {
                    Some(list) => list.iter().map(|s| (s.operand, s.face.0)).collect(),
                    None => result
                        .faces
                        .get(f as usize)
                        .map(|s| vec![(s.operand, s.face.0)])
                        .unwrap_or_default(),
                }
            }
            Bodies::Operands => {
                let operand = if body == 0 { Operand::A } else { Operand::B };
                vec![(operand, f)]
            }
        }
    }

    /// The faces of A and of B an entity lies on, each ascending.
    fn sides(&self, who: Who) -> (Vec<u32>, Vec<u32>) {
        let (mut on_a, mut on_b) = (Vec::new(), Vec::new());
        let Some(topo) = self.topos.get(who.0) else {
            return (on_a, on_b);
        };
        for f in topo.faces(who.1) {
            for (operand, face) in self.sources(who.0, f) {
                match operand {
                    Operand::A => on_a.push(face),
                    Operand::B => on_b.push(face),
                }
            }
        }
        for list in [&mut on_a, &mut on_b] {
            list.sort_unstable();
            list.dedup();
        }
        (on_a, on_b)
    }

    /// Whether a face of A one lies on and a face of B the other lies on
    /// were met by the section.
    fn met_across(&self, x: Who, y: Who) -> bool {
        let ((xa, xb), (ya, yb)) = (self.sides(x), self.sides(y));
        let any = |on_a: &[u32], on_b: &[u32]| {
            on_a.iter()
                .any(|&fa| on_b.iter().any(|&fb| self.met.contains(&(fa, fb))))
        };
        any(xa.as_slice(), yb.as_slice()) || any(ya.as_slice(), xb.as_slice())
    }

    /// The candidate pairs: entities on a face of A some pair of `met`
    /// holds, against entities on a face of B some pair holds, whose boxes
    /// grown by twice the tolerance meet; each pair once, ascending.
    fn pairs(&mut self) -> Vec<(Who, Who)> {
        let (mut on_a, mut on_b) = (BTreeSet::new(), BTreeSet::new());
        for &(fa, fb) in &self.met {
            on_a.insert(fa);
            on_b.insert(fb);
        }
        let mut left: Vec<Item> = Vec::new();
        let mut right: Vec<Item> = Vec::new();
        for (i, topo) in self.topos.iter().enumerate() {
            let body = topo.body;
            // Per face: whether it is a piece of a face of A, or of B, that
            // the section met.
            let flags: Vec<(bool, bool)> = (0..body.faces.len())
                .map(|f| {
                    self.sources(i, index(f)).into_iter().fold(
                        (false, false),
                        |(fa, fb), (operand, face)| match operand {
                            Operand::A => (fa || on_a.contains(&face), fb),
                            Operand::B => (fa, fb || on_b.contains(&face)),
                        },
                    )
                })
                .collect();
            let either = |list: &[u32], of: &[(bool, bool)]| {
                list.iter().fold((false, false), |(fa, fb), &k| {
                    let (ga, gb) = of.get(k as usize).copied().unwrap_or((false, false));
                    (fa || ga, fb || gb)
                })
            };
            let edge_flags: Vec<(bool, bool)> = (0..body.edges.len())
                .map(|e| either(topo.edge_faces.of(index(e)), flags.as_slice()))
                .collect();
            let mut push = |who: Who, bbox: Option<Bbox3>, (fa, fb): (bool, bool)| {
                let Some(bbox) = bbox else {
                    return;
                };
                if fa {
                    left.push(Item { who, bbox });
                }
                if fb {
                    right.push(Item { who, bbox });
                }
            };
            for (f, &flag) in flags.iter().enumerate() {
                if flag.0 || flag.1 {
                    let id = index(f);
                    push((i, Ent::Face(id)), body.face_bbox(FaceId(id)).ok(), flag);
                }
            }
            for (e, &flag) in edge_flags.iter().enumerate() {
                if flag.0 || flag.1 {
                    let id = index(e);
                    push((i, Ent::Edge(id)), body.edge_bbox(EdgeId(id)).ok(), flag);
                }
            }
            for (v, vertex) in body.vertices.iter().enumerate() {
                let id = index(v);
                let flag = either(topo.vertex_edges.of(id), edge_flags.as_slice());
                if (flag.0 || flag.1) && vertex.point.is_finite() {
                    push(
                        (i, Ent::Vertex(id)),
                        Some(Bbox3::from_points([vertex.point])),
                        flag,
                    );
                }
            }
        }
        if left.is_empty() || right.is_empty() {
            return Vec::new();
        }
        // A tree of the larger side's boxes, asked with each of the other's.
        let (few, many) = if left.len() <= right.len() {
            (&left, &right)
        } else {
            (&right, &left)
        };
        let boxes: Vec<Bbox3> = many.iter().map(|it| it.bbox).collect();
        let tree = FaceTree::new(&boxes);
        let near = self.near;
        let mut pairs: Vec<(Who, Who)> = Vec::new();
        for it in few {
            for k in tree.candidates(|bx| boxes_meet(bx, &it.bbox, near)) {
                let Some(other) = many.get(k) else {
                    continue;
                };
                if other.who == it.who || !boxes_meet(&other.bbox, &it.bbox, near) {
                    continue;
                }
                pairs.push(if it.who < other.who {
                    (it.who, other.who)
                } else {
                    (other.who, it.who)
                });
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        pairs
    }

    /// Measures each candidate pair that may be an approach.
    fn measure(&mut self, pairs: Vec<(Who, Who)>) -> Vec<Approach> {
        let mut found = Vec::new();
        let mut measured = 0usize;
        for (x, y) in pairs {
            let (x, y) = if rank(x.1) > rank(y.1) {
                (y, x)
            } else {
                (x, y)
            };
            if let (Ent::Vertex(_), Ent::Vertex(_)) = (x.1, y.1) {
                continue;
            }
            if x.0 == y.0 {
                let Some(topo) = self.topos.get_mut(x.0) else {
                    continue;
                };
                if topo.share_vertex(x.1, y.1) {
                    continue;
                }
            }
            if !self.met_across(x, y) {
                continue;
            }
            measured += 1;
            if measured > MOST_PAIRS {
                break;
            }
            let approach = match (x.1, y.1) {
                (Ent::Vertex(_), Ent::Edge(_)) => self.vertex_edge(x, y),
                (Ent::Vertex(_), Ent::Face(_)) => self.vertex_face(x, y),
                (Ent::Edge(_), Ent::Edge(_)) => self.edge_edge(x, y),
                (Ent::Edge(_), Ent::Face(_)) => self.edge_face(x, y),
                (Ent::Face(_), Ent::Face(_)) => self.face_face(x, y),
                _ => None,
            };
            if let Some(found_one) = approach {
                found.push(self.read(found_one));
            }
        }
        found
    }

    /// `approach` with its shells, whether it hops, and its faces read.
    fn read(&mut self, mut approach: Approach) -> Approach {
        let (x, y) = (approach.x, approach.y);
        let (first, mut second) = (self.named(x), self.named(y));
        if first.0 == second.0 {
            // Name one of each operand where there is one.
            let (xa, xb) = self.sides(x);
            let (ya, yb) = self.sides(y);
            let pick = |list: &[u32], operand: Operand| list.first().map(|&f| (operand, FaceId(f)));
            let other = match first.0 {
                Operand::A => {
                    pick(yb.as_slice(), Operand::B).or_else(|| pick(xb.as_slice(), Operand::B))
                }
                Operand::B => {
                    pick(ya.as_slice(), Operand::A).or_else(|| pick(xa.as_slice(), Operand::A))
                }
            };
            if let Some(other) = other {
                second = other;
            }
        }
        approach.faces = [first, second];
        if x.0 != y.0 {
            // Of two bodies (the operands): apart.
            approach.parts = true;
            return approach;
        }
        let Some(topo) = self.topos.get_mut(x.0) else {
            return approach;
        };
        approach.parts = topo.shell(x.1) != topo.shell(y.1);
        for ent in [x.1, y.1] {
            if let Ent::Face(f) = ent {
                topo.ensure_vertices(f);
            }
        }
        let (vx, vy) = (topo.vertices(x.1), topo.vertices(y.1));
        approach.hop = vx.iter().any(|&v| {
            topo.vertex_edges.of(v).iter().any(|&e| {
                topo.body.edges.get(e as usize).is_some_and(|edge| {
                    let w = if edge.start.0 == v {
                        edge.end.0
                    } else {
                        edge.start.0
                    };
                    vy.binary_search(&w).is_ok()
                })
            })
        });
        approach
    }

    /// A face of the operands an entity lies on.
    fn named(&self, who: Who) -> (Operand, FaceId) {
        let first = self
            .topos
            .get(who.0)
            .and_then(|topo| topo.faces(who.1).first().copied())
            .and_then(|f| self.sources(who.0, f).first().copied());
        match first {
            Some((operand, face)) => (operand, FaceId(face)),
            None => (Operand::A, FaceId(u32::MAX)),
        }
    }

    fn body(&self, who: Who) -> Option<&'r Body> {
        self.topos.get(who.0).map(|topo| topo.body)
    }

    fn point(&self, who: Who) -> Option<Vec3> {
        let Ent::Vertex(v) = who.1 else {
            return None;
        };
        let p = self.body(who)?.vertices.get(v as usize)?.point;
        p.is_finite().then_some(p)
    }

    fn edge(&self, who: Who) -> Option<&'r Edge> {
        let Ent::Edge(e) = who.1 else {
            return None;
        };
        self.body(who)?.edges.get(e as usize)
    }

    fn approach(x: Who, y: Who, p: Vec3, q: Vec3) -> Approach {
        Approach {
            x,
            y,
            p,
            q,
            distance: p.distance(q),
            parts: false,
            hop: false,
            faces: [(Operand::A, FaceId(u32::MAX)); 2],
        }
    }

    /// False only where the covers of the operands' faces that face `f`
    /// of body `body` is a piece of prove `p` further than twice the
    /// tolerance from all of them.
    fn covers_hold(&self, body: usize, f: u32, p: Vec3) -> bool {
        self.sources(body, f).into_iter().any(|(operand, face)| {
            self.solid(operand)
                .cover(face as usize, self.tol)
                .is_none_or(|(cover, of)| cover.holds(of, p, self.near))
        })
    }

    /// As [`Look::covers_hold`], for the box `bx`.
    fn covers_reach(&self, body: usize, f: u32, bx: &Bbox3) -> bool {
        self.sources(body, f).into_iter().any(|(operand, face)| {
            self.solid(operand)
                .cover(face as usize, self.tol)
                .is_none_or(|(cover, of)| cover.reaches(of, bx, None, self.near))
        })
    }

    /// False only where the covers prove every pair of faces of the
    /// operands (one of A, one of B, met by the section) that faces `f`
    /// of `x`'s body and `g` of `y`'s are pieces of further apart than
    /// twice the tolerance; `bf` and `bg` are their boxes.
    fn covers_meet(
        &self,
        (x, f): (usize, u32),
        (y, g): (usize, u32),
        bf: &Bbox3,
        bg: &Bbox3,
    ) -> bool {
        let (sx, sy) = (self.sources(x, f), self.sources(y, g));
        for &(ox, fx) in &sx {
            for &(oy, fy) in &sy {
                if ox == oy {
                    continue;
                }
                let key = if ox == Operand::A { (fx, fy) } else { (fy, fx) };
                if !self.met.contains(&key) {
                    continue;
                }
                let cx: Option<(&Cover, _)> = self.solid(ox).cover(fx as usize, self.tol);
                let cy: Option<(&Cover, _)> = self.solid(oy).cover(fy as usize, self.tol);
                let may = match (cx, cy) {
                    (Some((c1, o1)), Some((c2, o2))) => c1.meets(o1, c2, o2, self.near),
                    (Some((c1, o1)), None) => c1.reaches(o1, bg, None, self.near),
                    (None, Some((c2, o2))) => c2.reaches(o2, bf, None, self.near),
                    (None, None) => true,
                };
                if may {
                    return true;
                }
            }
        }
        false
    }

    fn vertex_edge(&self, x: Who, y: Who) -> Option<Approach> {
        let p = self.point(x)?;
        let edge = self.edge(y)?;
        let (t, d) = outline::nearest(&edge.curve, edge.range, p)?;
        if !(d.is_finite() && d < self.near) || at_end(t, edge.range) {
            return None;
        }
        let q = edge.curve.eval(t);
        (p.distance(q) < self.near).then(|| Self::approach(x, y, p, q))
    }

    fn vertex_face(&mut self, x: Who, y: Who) -> Option<Approach> {
        let p = self.point(x)?;
        let Ent::Face(f) = y.1 else {
            return None;
        };
        let face = self.body(y)?.faces.get(f as usize)?;
        let (near, tol) = (self.near, self.tol);
        if is_freeform(&face.surface) {
            if !self.covers_hold(y.0, f, p) {
                return None;
            }
        } else if outline::surface_distance(Sheet::bare(&face.surface), p) >= near {
            return None;
        }
        if !self.topos.get_mut(y.0)?.ensure(f, tol) {
            return None;
        }
        let o = self.topos.get(y.0)?.made(f)?;
        let q = outline::onto(o.sheet(), p)?;
        (p.distance(q) < near && inside(o, q, tol)).then(|| Self::approach(x, y, p, q))
    }

    fn edge_edge(&self, x: Who, y: Who) -> Option<Approach> {
        let (e1, e2) = (self.edge(x)?, self.edge(y)?);
        let (r1, r2) = (e1.range, e2.range);
        if !(r1.1 > r1.0 && r2.1 > r2.0) {
            return None;
        }
        let near = self.near;
        if let (Curve3::Line(_), Curve3::Line(_)) = (&e1.curve, &e2.curve) {
            let (p0, p1) = (e1.curve.eval(r1.0), e1.curve.eval(r1.1));
            let (q0, q1) = (e2.curve.eval(r2.0), e2.curve.eval(r2.1));
            let (s, t) = segments(p0, p1, q0, q1)?;
            if s <= END || s >= 1.0 - END || t <= END || t >= 1.0 - END {
                return None;
            }
            let (p, q) = (p0.lerp(p1, s), q0.lerp(q1, t));
            return (p.distance(q) < near).then(|| Self::approach(x, y, p, q));
        }
        // Along the edge whose nearest points are the dearer to find (a
        // spline), measured against the other.
        let flip = matches!(e2.curve, Curve3::Nurbs(_)) && !matches!(e1.curve, Curve3::Nurbs(_));
        let (along, other) = if flip { (e2, e1) } else { (e1, e2) };
        let distance = |pt: Vec3| {
            outline::nearest(&other.curve, other.range, pt).map_or(f64::INFINITY, |(_, d)| d)
        };
        let mut best: Option<Approach> = None;
        for t in least_along(&along.curve, along.range, near, distance) {
            if at_end(t, along.range) {
                continue;
            }
            let p = along.curve.eval(t);
            let Some((u, _)) = outline::nearest(&other.curve, other.range, p) else {
                continue;
            };
            if at_end(u, other.range) {
                continue;
            }
            let q = other.curve.eval(u);
            let d = p.distance(q);
            if d < near && best.is_none_or(|b| d < b.distance) {
                best = Some(if flip {
                    Self::approach(x, y, q, p)
                } else {
                    Self::approach(x, y, p, q)
                });
            }
        }
        best
    }

    fn edge_face(&mut self, x: Who, y: Who) -> Option<Approach> {
        let edge = self.edge(x)?;
        let (Ent::Edge(e), Ent::Face(f)) = (x.1, y.1) else {
            return None;
        };
        let face = self.body(y)?.faces.get(f as usize)?;
        // A straight edge over a flat face comes nearest at an end (a
        // vertex on the face) or is as near all along, and then as near
        // where it passes the face's boundary (edges of the face).
        if matches!(edge.curve, Curve3::Line(_)) && matches!(face.surface, Surface::Plane(_)) {
            return None;
        }
        let r = edge.range;
        if !(r.0.is_finite() && r.1 > r.0) {
            return None;
        }
        let (near, tol) = (self.near, self.tol);
        let bx = self.body(x)?.edge_bbox(EdgeId(e)).ok()?;
        if is_freeform(&face.surface) {
            if !self.covers_reach(y.0, f, &bx) {
                return None;
            }
        } else if let Some((lo, hi)) = bx.corners() {
            let middle = lo.lerp(hi, 0.5);
            if outline::surface_distance(Sheet::bare(&face.surface), middle)
                > 0.5 * lo.distance(hi) + near
            {
                return None;
            }
        }
        if !self.topos.get_mut(y.0)?.ensure(f, tol) {
            return None;
        }
        let o = self.topos.get(y.0)?.made(f)?;
        let sheet = o.sheet();
        let distance =
            |pt: Vec3| outline::onto(sheet, pt).map_or(f64::INFINITY, |q| pt.distance(q));
        let mut best: Option<Approach> = None;
        for t in least_along(&edge.curve, r, near, distance) {
            if at_end(t, r) {
                continue;
            }
            let p = edge.curve.eval(t);
            let Some(q) = outline::onto(sheet, p) else {
                continue;
            };
            let d = p.distance(q);
            if d < near && inside(o, q, tol) && best.is_none_or(|b| d < b.distance) {
                best = Some(Self::approach(x, y, p, q));
            }
        }
        best
    }

    fn face_face(&mut self, x: Who, y: Who) -> Option<Approach> {
        let (Ent::Face(f), Ent::Face(g)) = (x.1, y.1) else {
            return None;
        };
        let (body_x, body_y) = (self.body(x)?, self.body(y)?);
        let (ff, fg) = (body_x.faces.get(f as usize)?, body_y.faces.get(g as usize)?);
        // Two flat faces come nearest at a boundary (looked at as edges and
        // vertices) unless they are parallel, and then as near there.
        if matches!(ff.surface, Surface::Plane(_)) && matches!(fg.surface, Surface::Plane(_)) {
            return None;
        }
        let (near, tol) = (self.near, self.tol);
        // Each box's middle against the other surface, where it is known
        // exactly: the whole faces' boxes first, then (for a piece of a
        // ball or a ring) the boxes of what they hold.
        let far = |bx: &Bbox3, other: &Surface| {
            !is_freeform(other)
                && bx.corners().is_some_and(|(lo, hi)| {
                    outline::surface_distance(Sheet::bare(other), lo.lerp(hi, 0.5))
                        > 0.5 * lo.distance(hi) + near
                })
        };
        let whole_f = body_x.face_bbox(FaceId(f)).ok()?;
        let whole_g = body_y.face_bbox(FaceId(g)).ok()?;
        if far(&whole_f, &fg.surface) || far(&whole_g, &ff.surface) {
            return None;
        }
        let box_f = self.topos.get_mut(x.0)?.held_box(f)?;
        let box_g = self.topos.get_mut(y.0)?.held_box(g)?;
        if !boxes_meet(&box_f, &box_g, near) || far(&box_f, &fg.surface) || far(&box_g, &ff.surface)
        {
            return None;
        }
        let ((lo1, hi1), (lo2, hi2)) = (box_f.corners()?, box_g.corners()?);
        if (is_freeform(&ff.surface) || is_freeform(&fg.surface))
            && !self.covers_meet((x.0, f), (y.0, g), &box_f, &box_g)
        {
            return None;
        }
        let made_f = self.topos.get_mut(x.0)?.ensure(f, tol);
        let made_g = self.topos.get_mut(y.0)?.ensure(g, tol);
        if !(made_f && made_g) {
            return None;
        }
        let o1 = self.topos.get(x.0)?.made(f)?;
        let o2 = self.topos.get(y.0)?.made(g)?;
        let (common_lo, common_hi) = (lo1.max(lo2), hi1.min(hi2));
        let seeds = [
            common_lo.lerp(common_hi, 0.5),
            lo1.lerp(hi1, 0.5),
            lo2.lerp(hi2, 0.5),
        ];
        let mut best: Option<Approach> = None;
        for seed in seeds {
            if !seed.is_finite() {
                continue;
            }
            let Some(mut p) = outline::onto(o1.sheet(), seed) else {
                continue;
            };
            let mut q = p;
            let mut moved_any = false;
            for _ in 0..ALTERNATE {
                let Some(q2) = outline::onto(o2.sheet(), p) else {
                    break;
                };
                let Some(p2) = outline::onto(o1.sheet(), q2) else {
                    break;
                };
                let moved = p2.distance(p);
                p = p2;
                q = q2;
                moved_any = true;
                if moved <= 1e-6 * tol {
                    break;
                }
            }
            if !moved_any {
                continue;
            }
            let d = p.distance(q);
            if d < near
                && inside(o1, p, tol)
                && inside(o2, q, tol)
                && best.is_none_or(|b| d < b.distance)
            {
                best = Some(Self::approach(x, y, p, q));
            }
        }
        best
    }
}

/// Inside the face, or within the tolerance of its boundary and inside by
/// the sign of its offset from the nearest piece.
fn inside(o: &Outline, p: Vec3, tol: f64) -> bool {
    match o.side(p, tol) {
        Ok(Side::In) => true,
        Ok(Side::On) => o.inside_by_sign(p, tol) == Some(true),
        _ => false,
    }
}

fn at_end(t: f64, range: (f64, f64)) -> bool {
    let room = END * (range.1 - range.0).abs();
    t <= range.0 + room || t >= range.1 - room
}

/// The parameter in `(lo, hi)` where `f` is least, by golden sections.
fn golden(lo: f64, hi: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
    let (mut a, mut b) = (lo, hi);
    let mut c = b - INVERSE_GOLDEN * (b - a);
    let mut d = a + INVERSE_GOLDEN * (b - a);
    let (mut fc, mut fd) = (f(c), f(d));
    for _ in 0..GOLDEN {
        if !(fc.is_finite() || fd.is_finite()) {
            break;
        }
        if fc <= fd || !fd.is_finite() {
            b = d;
            d = c;
            fd = fc;
            c = b - INVERSE_GOLDEN * (b - a);
            fc = f(c);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + INVERSE_GOLDEN * (b - a);
            fd = f(d);
        }
    }
    if fc <= fd { c } else { d }
}

/// Where along `curve` over `range` the distance `distance` measures may
/// be least below `near`: the parameters found by golden sections in the
/// stretches between samples that may come that near (at most
/// [`SEARCHES`], the lowest bounds first), and any sample that is.
///
/// A stretch between samples `t0` and `t1` at distances `d0` and `d1` comes
/// no nearer than `(d0 + d1 − s (t1 − t0)) / 2`, `s` the curve's speed bound
/// ([`speed_bound`]): the distance changes by no more than the curve moves.
/// None where every stretch is proven further (the distance a true distance
/// to a set the other entity lies in).
fn least_along(
    curve: &Curve3,
    range: (f64, f64),
    near: f64,
    mut distance: impl FnMut(Vec3) -> f64,
) -> Vec<f64> {
    let (t0, t1) = range;
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return Vec::new();
    }
    let speed = speed_bound(curve);
    let step = (t1 - t0) / SAMPLES as f64;
    let ts: Vec<f64> = (0..=SAMPLES)
        .map(|k| {
            if k == SAMPLES {
                t1
            } else {
                t0 + step * k as f64
            }
        })
        .collect();
    let mut at = |t: f64| {
        let d = distance(curve.eval(t));
        if d.is_nan() { f64::INFINITY } else { d }
    };
    let ds: Vec<f64> = ts.iter().map(|&t| at(t)).collect();
    let mut open: Vec<(f64, usize)> = Vec::new();
    for (k, pair) in ds.windows(2).enumerate() {
        let &[d0, d1] = pair else {
            continue;
        };
        let bound = if speed.is_finite() {
            0.5 * (d0 + d1 - speed * step)
        } else {
            f64::NEG_INFINITY
        };
        if bound < near {
            open.push((bound, k));
        }
    }
    let mut found: Vec<f64> = ts
        .iter()
        .zip(&ds)
        .filter(|&(_, &d)| d < near)
        .map(|(&t, _)| t)
        .collect();
    open.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut searched: Vec<usize> = Vec::new();
    for &(_, k) in &open {
        if searched.len() >= SEARCHES {
            break;
        }
        if searched.iter().any(|&s| s.abs_diff(k) <= 1) {
            continue;
        }
        searched.push(k);
        let lo = ts.get(k.saturating_sub(1)).copied().unwrap_or(t0);
        let hi = ts.get(k + 2).copied().unwrap_or(t1);
        found.push(golden(lo, hi, &mut at));
    }
    found
}

/// The parameters of the points of two segments nearest each other, each
/// a share of its segment.
fn segments(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> Option<(f64, f64)> {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    if !(a > 0.0 && e > 0.0) {
        return None;
    }
    let c = d1.dot(r);
    let b = d1.dot(d2);
    let denom = a * e - b * b;
    let mut s = if denom > 1e-14 * a * e {
        ((b * f - c * e) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    (s.is_finite() && t.is_finite()).then_some((s, t))
}
