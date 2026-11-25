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
//!   `section::to_meet`), and keeps them (`Sections::met`), with the pairs
//!   its hulls proved apart by twice the tolerance but not by that and the
//!   merging distance more (`Sections::beside`: an entity of the result may
//!   lie that far off the face it is a piece of). An entity lies on the faces
//!   around it, and a face of the result on the faces of the operands it is
//!   a piece of, so two entities come that near only where a face of A
//!   around one and a face of B around the other are such a pair, or where
//!   both lie on faces of one operand: an approach that operand had already,
//!   not looked at. This is read from the sources before any geometry.
//! - Their boxes, grown by twice the tolerance, meet: a tree of the boxes of
//!   the larger side, asked with each box of the other in turn, each answer
//!   measured as it comes. A vertex or an edge is passed over against a
//!   piece of a ball or a ring where the box of what the piece holds is
//!   that far from it (since 2026-11-25, [`Look::away_from_piece`]).
//! - They share no vertex: two entities that share one anywhere are not
//!   measured, however near they come away from it (a face and its own edge;
//!   read from the topology alone, owed in the roadmap).
//! - Not two vertices: every vertex of a result is a point the section
//!   registered, and two of those in the band are refused there.
//!
//! A look does at most [`MOST_WORK`] units of work ([`Work`]: a pair looked
//! at, a point sought or measured; each unit's cost is bounded) and holds
//! at most [`MOST_FOUND`] approaches. A look that reaches either stops and
//! says so ([`Looked::finished`]), and the entities of the smaller side it
//! left are counted (`crate::work::near_left`; its work in
//! `crate::work::near_work`): a pair it did not measure may be an
//! approach, so the boolean refuses ([`CUT_SHORT`]) where what was found
//! does not refuse it already, and interference (`contact.rs` › `evidence`)
//! answers unknown where nothing was found, never clear (the rule of
//! `craft3d_geom::allowance`: a loop that stops never says "nothing
//! found"). The caps count work, not pairs (since 2026-11-25), so that
//! they bound the look's time and memory: a pair costs from a few units to
//! hundreds by its kinds, and the caps of pairs (2^21 measured, 2^24 looked
//! at, 2026-11-25) let a look of edges against edges run for minutes and
//! hold a quarter of a gigabyte of approaches before it stopped (the
//! review of the dimple and the caps, F7). The largest look measured did
//! 16 105 070 units (the engine's fuzz findings, a pattern of shelled
//! tubes less a ball: 7.5 s in release on a busy machine, half a
//! microsecond a unit; the large looks 0.2 to 1.3 µs a unit), the
//! benchmark's at most 8 108 and the corpus's 28 279; the cap is four
//! times the largest: at the units' costs averaged over those large looks
//! some 15 to 90 s of such work (an average over the workloads measured,
//! not a bound: a unit's own cost is bounded by its search's caps, which
//! allow more). The 65 536 approaches, the joined ones (left as they are)
//! counted with the rest, hold some 7 MB (none of those looks found one).
//!
//! **How near.** A vertex to an edge or a face by its nearest point; an edge
//! to an edge or a face by its distance sampled along the edge, each stretch
//! between samples bounded below by the edge's speed, and only the stretches
//! that may come within twice the tolerance searched closer (golden
//! sections). Two edges are first bounded from below on both sides
//! ([`edges_apart`], since 2026-11-25: a line by its chord, an arc of a
//! circle or an ellipse of at most half a turn by its chord and sagitta, a
//! piece of a spline by its chord and the control points of the spans it
//! crosses (their convex hull) or by its middle and its speed, any other
//! piece by its middle and its speed, halved where they may come that
//! near; never a spline edge whose range runs past its curve's domain),
//! and passed over, before any nearest point is sought, where every pair
//! of their stretches is proven further apart. Two faces ([`least_between`]): where the kinds of their
//! surfaces have them in closed form (a plane or a ball against any
//! analytic surface, two cylinders: [`square_pairs`]), the pairs of points
//! on a line square to both surfaces. The least distance between points
//! inside both faces, neither on a boundary, is the distance of such a pair
//! (it is stationary there), or of a line or a circle of them (a rod lying
//! over a plane, a ball on a pipe's axis, a ring lying flat over a plane):
//! a line reaches a boundary of a face, which is looked at as its edges and
//! vertices, and a circle either does too or lies wholly inside both faces,
//! so one pair stands for it. For the other kinds (two of a cylinder, a cone
//! and a torus but two cylinders, and a freeform face against anything),
//! alternating projections from three seeds, and where they have not
//! settled in their steps a descent ([`descend`]): the projections crawl
//! where two faces are nearly tangent, by about `1 − gap · κ` a step (κ the
//! difference of the faces' curvatures; a ball of radius 20 a tolerance and
//! a half over a plane, 0.04 % a step), and a line search takes the step
//! the distance asks for. Where they settled, on a pair square to both
//! surfaces that may be a saddle of the distance (a ring slid on a rod off
//! its axis: least across, greatest round), the distance's curvature
//! across the first surface, and where it curves down some way a line
//! search that way and a descent from there ([`escape`], since
//! 2026-11-25); and a cone's apex with its nearest point. For
//! every kind also the search as it was before (the projections' nearest
//! end within twice the tolerance, polished by golden sections across the
//! first face). A straight edge over a plane and two planes come nearest at
//! a boundary, which is looked at as its own edges and vertices. Before any
//! of that, a pair is passed over where the other surface is far from the
//! box of an analytic entity, or where the covers of the operands' faces
//! prove a freeform face that far. An approach counts only where two points
//! of the two entities, each evaluated exactly, are less than twice the
//! tolerance apart, inside both: a search that misses misses, it never
//! finds an approach that is not there. The distance it gives is the least
//! found, a bound from above of the least there is: for two faces whose
//! kinds are in closed form, the least between their insides away from
//! their boundaries (at a boundary, the edges' and the vertices' measure);
//! for the others what the projections, the descents and the polish reach
//! from the seeds (measured against dense sampling for a spline face over a
//! plane and under a ball: the tests of this module).
//!
//! **What is done with one** ([`super::near_approaches`], and `contact`'s
//! evidence): two parts that come that near touch (zero thickness, the
//! developer's rule of 2026-10-15). Two walls of one shell are read at the
//! place between the two points as `within_a_shell` reads two faces at a
//! place (2026-10-18): the walls are the faces around each entity but a
//! face around both (the strip of a face the approach runs across, joined
//! to its neighbours by the entities' own edges whatever the walls do);
//! where a wall around the one and a wall around the other have a vertex in
//! common within four times the tolerance of the place, an edge in common
//! that passes that near, or an edge in common whose wedge reaches it
//! (`wedged`: within twice the tolerance over the sine of half the faces'
//! angle at the edge, up to a tenth of the smaller face, and beyond it
//! where the gap is at least half what the faces' angle leaves there), they
//! are joined there (a wedge, a wall that thins to a knife edge, a corner
//! of a face sharper than the tolerance resolves) and left as they are
//! ([`Approach::joined`]); otherwise, between half and twice the tolerance
//! apart, they are refused as too near to tell apart (2026-10-22): a web
//! that thin, a strip of a face that narrow. `walls_apart` and the reading
//! through a third face of `within_a_shell` are not asked. Measured on the
//! benchmark's models on 2026-11-24: about a hundredth of the booleans' own
//! time (docs/design/booleans.md › *Near approaches inside an edge or a
//! face*).

use super::cover::Cover;
use super::facetree::FaceTree;
use super::freeform::is_freeform;
use super::hits::speed_bound;
use super::outline::{self, Outline, Sheet, Side, boxes_meet};
use super::regions::ANGLE_GUARD;
use super::section::{self, Sections, Solid};
use super::{BooleanResult, Operand, Reason};
use craft3d_geom::{Cone, Curve3, Cylinder, NurbsCurve, Plane, Sphere, Surface};
use craft3d_math::{Bbox3, Portable, Vec3};
use craft3d_topo::{Body, Edge, EdgeId, FaceId};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// Samples along an edge.
const SAMPLES: usize = 24;
/// Stretches of an edge searched closer, at most, per pair.
const SEARCHES: usize = 3;
/// Steps of a golden-section search.
const GOLDEN: usize = 32;
/// Steps of the alternating projection between two faces.
const ALTERNATE: usize = 40;
/// Line searches of a descent between two faces ([`descend`]), at most.
const DESCENT: usize = 8;
/// The step of [`escape`]'s second differences, as a share of its reach.
const ESCAPE: f64 = 0.125;
/// A parameter this close to an end of its range (as a share of the
/// range) is at that end: the vertex, looked at as one.
const END: f64 = 1e-7;
/// Vertices and edges away from the boundary of a piece of a ball or a
/// ring measured against it before the box of what it holds is made and
/// read ([`Look::away_from_piece`]).
const HELD_AFTER: u32 = 8;
/// Pairs of stretches of two edges compared by [`edges_apart`], at most,
/// before it gives up and the edges are searched as before.
const APART: usize = 32;
/// The look's work, at most, in units of [`Work`] (a pair looked at, a
/// point sought or measured): past that the look stops, said
/// ([`Looked::finished`]) and counted (`crate::work::near_left`).
const MOST_WORK: u64 = 1 << 26;
/// Approaches held by one look, at most, joined ones ([`Approach::joined`],
/// which the boolean leaves as they are) counted with the rest: as
/// [`MOST_WORK`].
const MOST_FOUND: usize = 1 << 16;
/// Why a boolean is refused, and interference is unknown, where a look
/// stopped at a cap ([`MOST_WORK`], [`MOST_FOUND`]) and what it found
/// before does not refuse it already: what it did not measure may be too
/// near. The numbers are the caps' (a test holds them so).
pub(super) const CUT_SHORT: Reason = "the look for parts or walls closer together than the tolerance can tell apart stopped at its cap (67108864 units of work, or 65536 approaches found) before it had looked at every pair: what it left may be that near (a body too large to look at in one boolean)";
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
    /// In one shell, joined at the place between the two points by a face
    /// around each: a vertex or an edge in common there, or an edge in
    /// common whose wedge reaches it (the readings of `within_a_shell`,
    /// 2026-10-18; see the module). Left as it is.
    pub joined: bool,
    /// A face of the operands at each, for naming the pair: one of A and
    /// one of B where there are.
    pub faces: [(Operand, FaceId); 2],
}

/// What one look found, and whether it looked at every pair it had to.
#[derive(Clone, Debug)]
pub(super) struct Looked {
    /// The approaches found, in a fixed order.
    pub approaches: Vec<Approach>,
    /// False where the look stopped at a cap ([`MOST_WORK`] units of
    /// work, [`MOST_FOUND`] approaches) with pairs left: those may be
    /// approaches, so what the look did not find is not known to be clear
    /// ([`CUT_SHORT`]).
    pub finished: bool,
}

/// The work of one look, counted as it is done, for its cap
/// ([`MOST_WORK`]) and `crate::work::near_work`: one unit for each pair
/// looked at (its filters), each point of an edge's curve evaluated for
/// [`edges_apart`], and each point sought or measured on a curve or a
/// surface (a nearest point, a projection, a distance, a normal, a point
/// placed inside or outside a face, a cover asked). Each unit's cost is
/// bounded (every search has its own cap), so the cap bounds the look's
/// time; the work made once per entity (outlines, boxes, covers, the
/// faces an entity lies on) is bounded by the bodies' size.
struct Work(Cell<u64>);

impl Work {
    fn new() -> Work {
        Work(Cell::new(0))
    }

    fn add(&self, units: u64) {
        self.0.set(self.0.get().saturating_add(units));
    }

    fn done(&self) -> u64 {
        self.0.get()
    }

    fn onto(&self, sheet: Sheet, p: Vec3) -> Option<Vec3> {
        self.add(1);
        outline::onto(sheet, p)
    }

    fn nearest(&self, curve: &Curve3, range: (f64, f64), p: Vec3) -> Option<(f64, f64)> {
        self.add(1);
        outline::nearest(curve, range, p)
    }

    fn surface_distance(&self, sheet: Sheet, p: Vec3) -> f64 {
        self.add(1);
        outline::surface_distance(sheet, p)
    }

    fn normal(&self, sheet: Sheet, p: Vec3) -> Option<Vec3> {
        self.add(1);
        outline::outward_normal(sheet, false, p)
    }

    fn inside(&self, o: &Outline, p: Vec3, tol: f64) -> bool {
        self.add(1);
        inside(o, p, tol)
    }

    fn eval(&self, curve: &Curve3, t: f64) -> Vec3 {
        self.add(1);
        curve.eval(t)
    }
}

impl Looked {
    /// Every pair looked at: what was found is all there is to find.
    fn through(approaches: Vec<Approach>) -> Looked {
        Looked {
            approaches,
            finished: true,
        }
    }
}

/// The approaches in a boolean's result (one body): pairs of its entities
/// of which one lies on a face of A and the other on a face of B (see the
/// module), with `sections` what the section met. In a fixed order.
pub(super) fn in_result<'r, 'x>(
    result: &'r BooleanResult,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    sections: &Sections,
    tol: f64,
) -> Looked {
    look(
        Bodies::Result(result),
        vec![&result.body],
        a,
        b,
        sections,
        tol,
    )
}

/// The approaches between the two operands themselves: pairs of an entity
/// of A (body 0) and one of B (body 1).
pub(super) fn between<'r, 'x>(
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    sections: &Sections,
    tol: f64,
) -> Looked {
    look(Bodies::Operands, vec![a.body, b.body], a, b, sections, tol)
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
    /// Per face, its edges, made when first asked for.
    face_edges: Vec<Option<Vec<u32>>>,
    /// Per face, its outline, made when first asked for.
    outlines: Vec<Option<Option<Outline>>>,
    /// Per face, the box of what it holds (`Body::held_face_bbox`), made
    /// when first asked for: for a piece of a ball or a ring much smaller
    /// than the box of the whole, which the pairing takes (dearer to make,
    /// so made only for the faces a pair of faces is measured on, and those
    /// [`Look::away_from_piece`] asks).
    held: Vec<Option<Option<Bbox3>>>,
    /// Per face, the box of its boundary (its loops), made when first
    /// asked for.
    rims: Vec<Option<Option<Bbox3>>>,
    /// Per face, how many vertices and edges away from its boundary were
    /// measured against it ([`Look::away_from_piece`]).
    away: Vec<u32>,
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
            face_edges: vec![None; body.faces.len()],
            outlines: (0..body.faces.len()).map(|_| None).collect(),
            held: vec![None; body.faces.len()],
            rims: vec![None; body.faces.len()],
            away: vec![0; body.faces.len()],
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

    /// The vertices of an entity, ascending, each once.
    fn vertices(&mut self, ent: Ent) -> Vec<u32> {
        match ent {
            Ent::Vertex(v) => vec![v],
            Ent::Edge(e) => match self.body.edges.get(e as usize) {
                Some(edge) => {
                    let mut v = vec![edge.start.0, edge.end.0];
                    v.sort_unstable();
                    v.dedup();
                    v
                }
                None => Vec::new(),
            },
            Ent::Face(f) => {
                self.ensure_vertices(f);
                self.face_vertex_list(f).to_vec()
            }
        }
    }

    /// Makes the list of face `f`'s edges if it is not made yet.
    fn ensure_edges(&mut self, f: u32) {
        let body = self.body;
        let Some(slot) = self.face_edges.get_mut(f as usize) else {
            return;
        };
        if slot.is_some() {
            return;
        }
        let mut out: Vec<u32> = body
            .faces
            .get(f as usize)
            .into_iter()
            .flat_map(|face| &face.loops)
            .filter_map(|l| body.loop_(*l))
            .flat_map(|lp| lp.coedges.iter().map(|c| c.edge.0))
            .collect();
        out.sort_unstable();
        out.dedup();
        *slot = Some(out);
    }

    /// The vertices of face `f`, once [`Topo::ensure_vertices`] made them.
    fn face_vertex_list(&self, f: u32) -> &[u32] {
        self.face_vertices
            .get(f as usize)
            .and_then(|list| list.as_deref())
            .unwrap_or(&[])
    }

    /// The edges of face `f`, once [`Topo::ensure_edges`] made them.
    fn face_edge_list(&self, f: u32) -> &[u32] {
        self.face_edges
            .get(f as usize)
            .and_then(|list| list.as_deref())
            .unwrap_or(&[])
    }

    /// Whether two entities of one shell are joined at `place`, `gap`
    /// apart, as `within_a_shell` reads two faces at a place (2026-10-18):
    /// a wall around the one and a wall around the other with a vertex in
    /// common within four times the tolerance of the place, an edge in
    /// common that passes that near, or an edge in common whose wedge
    /// reaches it ([`Topo::wedged`]). The walls are the faces around each
    /// but those around both: a face whose boundary holds both entities is
    /// the one the approach runs across (a strip of it that narrow), and is
    /// joined to the faces on either side of it by the entities' own edges
    /// whatever the walls beside the strip do.
    fn joined_at(&mut self, x: Ent, y: Ent, place: Vec3, gap: f64, near: f64) -> bool {
        let (fx, fy) = (self.faces(x), self.faces(y));
        for &f in fx.iter().chain(&fy) {
            self.ensure_vertices(f);
            self.ensure_edges(f);
        }
        let walls = |own: &[u32], other: &[u32]| -> Vec<u32> {
            own.iter()
                .copied()
                .filter(|f| other.binary_search(f).is_err())
                .collect()
        };
        let (wx, wy) = (
            walls(fx.as_slice(), fy.as_slice()),
            walls(fy.as_slice(), fx.as_slice()),
        );
        wx.iter().any(|&f| {
            wy.iter()
                .any(|&g| f != g && self.faces_joined(f, g, place, gap, near))
        })
    }

    /// [`Topo::joined_at`] for faces `f` and `g`.
    fn faces_joined(&self, f: u32, g: u32, place: Vec3, gap: f64, near: f64) -> bool {
        let body = self.body;
        let reach = 2.0 * near;
        let by_vertex = common(self.face_vertex_list(f), self.face_vertex_list(g))
            .iter()
            .any(|&v| {
                body.vertices
                    .get(v as usize)
                    .is_some_and(|vertex| vertex.point.distance(place) <= reach)
            });
        if by_vertex {
            return true;
        }
        let edges = common(self.face_edge_list(f), self.face_edge_list(g));
        !edges.is_empty() && self.wedged(f, g, &edges, place, gap, near)
    }

    /// Whether an edge `f` and `g` have in common joins them at `place`:
    /// passing within four times the tolerance of it, or with `place` in
    /// the wedge the two faces make at it, as `within_a_shell` › `wedged`
    /// reads it: within `2t / sin(θ/2)` of the edge (θ the angle between
    /// the faces, from their outward normals at the edge's point nearest the
    /// place, which is its foot or its nearer end) up to a tenth of the
    /// smaller face, and beyond that where the faces open (`sin(θ/2)` above
    /// `ANGLE_GUARD`), the place is further from the edge than four times
    /// the tolerance and the gap there is at least half of what two planes
    /// leaving the edge at θ have (`gap ≥ d sin(θ/2)`).
    fn wedged(&self, f: u32, g: u32, edges: &[u32], place: Vec3, gap: f64, near: f64) -> bool {
        let body = self.body;
        let reach = 2.0 * near;
        let size = |h: u32| body.face_bbox(FaceId(h)).map_or(0.0, |b| b.diagonal());
        let most = (0.1 * size(f).min(size(g))).max(reach);
        let (Some(sf), Some(sg)) = (body.faces.get(f as usize), body.faces.get(g as usize)) else {
            return false;
        };
        edges.iter().any(|&e| {
            let Some(edge) = body.edges.get(e as usize) else {
                return false;
            };
            let Some((t, _)) = outline::nearest(&edge.curve, edge.range, place) else {
                return false;
            };
            let q = edge.curve.eval(t);
            let d = q.distance(place);
            let boxed = body.edge_bbox(EdgeId(e)).ok();
            if boxed.is_some_and(|b| b.contains(place, reach)) && d <= reach {
                return true;
            }
            let within = boxed.is_some_and(|b| b.contains(place, most));
            match (sf.outward(q), sg.outward(q)) {
                (Some(nf), Some(ng)) => {
                    let half = (0.5 * (1.0 + nf.dot(ng))).max(0.0).sqrt();
                    let margin = if half > 0.0 {
                        near / half
                    } else {
                        f64::INFINITY
                    };
                    (within && d <= margin.max(reach).min(most))
                        || (half > ANGLE_GUARD && d > reach && d <= margin && gap >= d * half)
                }
                // No normal there (an apex): the widest margin, no further.
                _ => within && d <= most,
            }
        })
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

    /// The box of face `f`'s boundary ([`Topo::rims`]): its loops' boxes
    /// together; none for a face without loops, and none where an edge of
    /// its boundary is a spline whose range runs past its curve's domain
    /// ([`past_domain`]: that edge's box, and so the face's, need not hold
    /// it).
    fn rim_box(&mut self, f: u32) -> Option<Bbox3> {
        let body = self.body;
        let slot = self.rims.get_mut(f as usize)?;
        *slot.get_or_insert_with(|| {
            let face = body.faces.get(f as usize)?;
            let mut out = Bbox3::EMPTY;
            for &l in &face.loops {
                let lp = body.loop_(l)?;
                for c in &lp.coedges {
                    let edge = body.edge(c.edge)?;
                    if past_domain(&edge.curve, edge.range) {
                        return None;
                    }
                }
                let (lo, hi) = body.loop_bbox(l).ok()?.corners()?;
                out.add(lo);
                out.add(hi);
            }
            (!out.is_empty()).then_some(out)
        })
    }

    /// The outline of face `f` when [`Topo::ensure`] made it.
    fn made(&self, f: u32) -> Option<&Outline> {
        self.outlines.get(f as usize)?.as_ref()?.as_ref()
    }
}

/// An entity with its box, for the pairing, and what the filters read of
/// it ([`Info`], at `info` in the list [`Look::items`] makes).
struct Item {
    who: Who,
    bbox: Bbox3,
    info: usize,
}

/// What the filters of a pair read of an entity, read once per entity
/// rather than per pair: the faces of A and of B it lies on (each
/// ascending; [`Look::sides`]) and its vertices (ascending).
struct Info {
    on_a: Vec<u32>,
    on_b: Vec<u32>,
    vertices: Vec<u32>,
}

/// What one look knows.
struct Look<'r, 'x> {
    whence: Bodies<'r>,
    topos: Vec<Topo<'r>>,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    /// The pairs of faces (of A, of B) the section met, and those beside
    /// them (`Sections::beside`).
    met: BTreeSet<(u32, u32)>,
    tol: f64,
    near: f64,
    work: Work,
}

fn look<'r, 'x>(
    whence: Bodies<'r>,
    bodies: Vec<&'r Body>,
    a: &'r Solid<'x>,
    b: &'r Solid<'x>,
    sections: &Sections,
    tol: f64,
) -> Looked {
    let near = 2.0 * tol;
    if (sections.met.is_empty() && sections.beside.is_empty()) || !(near.is_finite() && near > 0.0)
    {
        return Looked::through(Vec::new());
    }
    let met: BTreeSet<(u32, u32)> = sections
        .met
        .iter()
        .chain(&sections.beside)
        .map(|&(fa, fb)| (index(fa), index(fb)))
        .collect();
    let mut look = Look {
        whence,
        topos: bodies.into_iter().map(Topo::new).collect(),
        a,
        b,
        met,
        tol,
        near,
        work: Work::new(),
    };
    let looked = look.run();
    crate::work::near_worked(look.work.done());
    looked
}

/// The caps of one look: its work, the approaches it holds.
#[cfg(not(test))]
fn caps() -> (u64, usize) {
    (MOST_WORK, MOST_FOUND)
}

/// The caps of one look, or those a test of this thread set.
#[cfg(test)]
fn caps() -> (u64, usize) {
    tests::CAPS
        .with(Cell::get)
        .unwrap_or((MOST_WORK, MOST_FOUND))
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
    fn met_across(&self, x: &Info, y: &Info) -> bool {
        let any = |on_a: &[u32], on_b: &[u32]| {
            on_a.iter()
                .any(|&fa| on_b.iter().any(|&fb| self.met.contains(&(fa, fb))))
        };
        any(&x.on_a, &y.on_b) || any(&y.on_a, &x.on_b)
    }

    /// The box an entity is paired by: its own (`Body::face_bbox`, which
    /// for a face on a ball or a ring is the whole ball's or ring's,
    /// `Body::edge_bbox`, its point).
    fn pairing_box(&self, who: Who) -> Option<Bbox3> {
        let body = self.topos.get(who.0)?.body;
        match who.1 {
            Ent::Face(f) => body.face_bbox(FaceId(f)).ok(),
            Ent::Edge(e) => body.edge_bbox(EdgeId(e)).ok(),
            Ent::Vertex(v) => {
                let point = body.vertices.get(v as usize)?.point;
                point.is_finite().then(|| Bbox3::from_points([point]))
            }
        }
    }

    /// Whether a vertex or an edge whose box is `bx` is proven further than
    /// twice the tolerance from face `f` of body `body`, a piece of a ball
    /// or a ring, by the box of what the face holds (since 2026-11-25). The
    /// pairing boxes such a face by the whole ball, so every entity near the
    /// ball meets every piece of it (the gate's twenty shelled balls: 1.1
    /// million pairs looked at, 164 497 of an edge and a face sampled).
    /// Asked only where `bx` is that far from the box of the face's boundary
    /// (no box of what the face holds is smaller), and only once
    /// [`HELD_AFTER`] such vertices and edges have been measured against the
    /// face: the box costs from a tenth of a millisecond to a few hundred on
    /// a face with a long boundary (some of the corpus's), more than a few
    /// pairs' measures. Grown, because a point the look takes as inside a
    /// face may lie up to the tolerance outside its boundary ([`inside`]),
    /// so up to that outside the box of what it holds: a pair passed over
    /// is one whose measure finds nothing. Never asked for an edge, nor of a
    /// face with an edge in its boundary, that is a spline whose range runs
    /// past its curve's domain ([`past_domain`]: its box need not hold it).
    fn away_from_piece(&mut self, body: usize, f: u32, bx: &Bbox3) -> bool {
        let reach = 2.0 * self.near;
        let Some(topo) = self.topos.get_mut(body) else {
            return false;
        };
        let shape = topo.body;
        let Some(face) = shape.faces.get(f as usize) else {
            return false;
        };
        if face.loops.is_empty() || !matches!(face.surface, Surface::Sphere(_) | Surface::Torus(_))
        {
            return false;
        }
        match topo.rim_box(f) {
            Some(rim) if !boxes_meet(bx, &rim, reach) => {}
            _ => return false,
        }
        let Some(away) = topo.away.get_mut(f as usize) else {
            return false;
        };
        *away = away.saturating_add(1);
        if *away < HELD_AFTER {
            return false;
        }
        topo.held_box(f)
            .is_some_and(|held| !boxes_meet(bx, &held, reach))
    }

    /// The two sides: the entities on a face of A some pair of `met` holds,
    /// and those on a face of B some pair holds, with their boxes
    /// ([`Look::pairing_box`]), and what the filters read of each entity
    /// (indexed by the items' `info`).
    fn items(&mut self) -> (Vec<Item>, Vec<Item>, Vec<Info>) {
        let (mut on_a, mut on_b) = (BTreeSet::new(), BTreeSet::new());
        for &(fa, fb) in &self.met {
            on_a.insert(fa);
            on_b.insert(fb);
        }
        // Per body: its faces, then its edges, then its vertices.
        let mut wanted: Vec<(Who, (bool, bool))> = Vec::new();
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
            for (f, &flag) in flags.iter().enumerate() {
                if flag.0 || flag.1 {
                    wanted.push(((i, Ent::Face(index(f))), flag));
                }
            }
            for (e, &flag) in edge_flags.iter().enumerate() {
                if flag.0 || flag.1 {
                    wanted.push(((i, Ent::Edge(index(e))), flag));
                }
            }
            for v in 0..body.vertices.len() {
                let id = index(v);
                let flag = either(topo.vertex_edges.of(id), edge_flags.as_slice());
                if flag.0 || flag.1 {
                    wanted.push(((i, Ent::Vertex(id)), flag));
                }
            }
        }
        let mut left: Vec<Item> = Vec::new();
        let mut right: Vec<Item> = Vec::new();
        let mut infos: Vec<Info> = Vec::with_capacity(wanted.len());
        for (who, (fa, fb)) in wanted {
            let Some(bbox) = self.pairing_box(who) else {
                continue;
            };
            let (on_a, on_b) = self.sides(who);
            let vertices = self
                .topos
                .get_mut(who.0)
                .map(|topo| topo.vertices(who.1))
                .unwrap_or_default();
            let info = infos.len();
            infos.push(Info {
                on_a,
                on_b,
                vertices,
            });
            if fa {
                left.push(Item { who, bbox, info });
            }
            if fb {
                right.push(Item { who, bbox, info });
            }
        }
        (left, right, infos)
    }

    /// The approaches: each entity of the smaller side against those of the
    /// larger side's tree whose grown boxes meet its own, measured as the
    /// tree gives them; in that order, which is fixed. Past [`MOST_WORK`]
    /// units of work or [`MOST_FOUND`] approaches the look stops, not
    /// finished, and the entities of the smaller side not finished are
    /// counted (`crate::work::near_left`).
    fn run(&mut self) -> Looked {
        let (left, right, infos) = self.items();
        if left.is_empty() || right.is_empty() {
            return Looked::through(Vec::new());
        }
        let (few, many) = if left.len() <= right.len() {
            (left, right)
        } else {
            (right, left)
        };
        // An entity on both sides (one the boolean made) meets another such
        // from either side: measured from the lower of the two only.
        let in_few: BTreeSet<Who> = few.iter().map(|it| it.who).collect();
        let in_many: BTreeSet<Who> = many.iter().map(|it| it.who).collect();
        let boxes: Vec<Bbox3> = many.iter().map(|it| it.bbox).collect();
        let tree = FaceTree::new(&boxes);
        let near = self.near;
        let (most_work, most_found) = caps();
        let mut found = Vec::new();
        for (done, it) in few.iter().enumerate() {
            for k in tree.candidates(|bx| boxes_meet(bx, &it.bbox, near)) {
                let Some(other) = many.get(k) else {
                    continue;
                };
                if other.who == it.who || !boxes_meet(&other.bbox, &it.bbox, near) {
                    continue;
                }
                if in_many.contains(&it.who) && in_few.contains(&other.who) && other.who < it.who {
                    continue;
                }
                self.work.add(1);
                if self.work.done() > most_work || found.len() >= most_found {
                    crate::work::near_left_over(few.len().saturating_sub(done));
                    return Looked {
                        approaches: found,
                        finished: false,
                    };
                }
                let (Some(x), Some(y)) = (infos.get(it.info), infos.get(other.info)) else {
                    continue;
                };
                if let Some(approach) = self.pair((it, x), (other, y)) {
                    found.push(approach);
                }
            }
        }
        Looked::through(found)
    }

    /// One pair whose grown boxes meet: the filters (two vertices, a
    /// vertex in common, no face of A and face of B met across, a vertex or
    /// an edge away from a piece of a ball or a ring: [`Look::away_from_piece`]),
    /// then its measure.
    fn pair(&mut self, x: (&Item, &Info), y: (&Item, &Info)) -> Option<Approach> {
        let (x, y) = if (rank(x.0.who.1), x.0.who) > (rank(y.0.who.1), y.0.who) {
            (y, x)
        } else {
            (x, y)
        };
        let ((xi, ix), (yi, iy)) = (x, y);
        let (x, y) = (xi.who, yi.who);
        if let (Ent::Vertex(_), Ent::Vertex(_)) = (x.1, y.1) {
            return None;
        }
        if x.0 == y.0 && share(&ix.vertices, &iy.vertices) {
            return None;
        }
        if !self.met_across(ix, iy) {
            return None;
        }
        let boxed = self
            .edge(x)
            .is_none_or(|edge| !past_domain(&edge.curve, edge.range));
        if let (Ent::Vertex(_) | Ent::Edge(_), Ent::Face(f)) = (x.1, y.1)
            && boxed
            && self.away_from_piece(y.0, f, &xi.bbox)
        {
            return None;
        }
        let approach = match (x.1, y.1) {
            (Ent::Vertex(_), Ent::Edge(_)) => self.vertex_edge(x, y),
            (Ent::Vertex(_), Ent::Face(_)) => self.vertex_face(x, y),
            (Ent::Edge(_), Ent::Edge(_)) => self.edge_edge(x, y),
            (Ent::Edge(_), Ent::Face(_)) => self.edge_face(x, y),
            (Ent::Face(_), Ent::Face(_)) => self.face_face(x, y),
            _ => None,
        }?;
        Some(self.read(approach))
    }

    /// `approach` with its shells, whether it is joined, and its faces read.
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
        let near = self.near;
        let Some(topo) = self.topos.get_mut(x.0) else {
            return approach;
        };
        approach.parts = topo.shell(x.1) != topo.shell(y.1);
        if !approach.parts {
            let place = approach.p.lerp(approach.q, 0.5);
            approach.joined = topo.joined_at(x.1, y.1, place, approach.distance, near);
        }
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
            joined: false,
            faces: [(Operand::A, FaceId(u32::MAX)); 2],
        }
    }

    /// False only where the covers of the operands' faces that face `f`
    /// of body `body` is a piece of prove `p` further than twice the
    /// tolerance from all of them.
    fn covers_hold(&self, body: usize, f: u32, p: Vec3) -> bool {
        self.work.add(1);
        self.sources(body, f).into_iter().any(|(operand, face)| {
            self.solid(operand)
                .cover(face as usize, self.tol)
                .is_none_or(|(cover, of)| cover.holds(of, p, self.near))
        })
    }

    /// As [`Look::covers_hold`], for the box `bx`.
    fn covers_reach(&self, body: usize, f: u32, bx: &Bbox3) -> bool {
        self.work.add(1);
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
        self.work.add(1);
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
        let (t, d) = self.work.nearest(&edge.curve, edge.range, p)?;
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
        } else if self.work.surface_distance(Sheet::bare(&face.surface), p) >= near {
            return None;
        }
        if !self.topos.get_mut(y.0)?.ensure(f, tol) {
            return None;
        }
        let o = self.topos.get(y.0)?.made(f)?;
        let q = self.work.onto(o.sheet(), p)?;
        (p.distance(q) < near && self.work.inside(o, q, tol)).then(|| Self::approach(x, y, p, q))
    }

    fn edge_edge(&self, x: Who, y: Who) -> Option<Approach> {
        let (e1, e2) = (self.edge(x)?, self.edge(y)?);
        let (r1, r2) = (e1.range, e2.range);
        if !(r1.1 > r1.0 && r2.1 > r2.0) {
            return None;
        }
        let near = self.near;
        let work = &self.work;
        if let (Curve3::Line(_), Curve3::Line(_)) = (&e1.curve, &e2.curve) {
            let (p0, p1) = (work.eval(&e1.curve, r1.0), work.eval(&e1.curve, r1.1));
            let (q0, q1) = (work.eval(&e2.curve, r2.0), work.eval(&e2.curve, r2.1));
            let (s, t) = segments(p0, p1, q0, q1)?;
            if s <= END || s >= 1.0 - END || t <= END || t >= 1.0 - END {
                return None;
            }
            let (p, q) = (p0.lerp(p1, s), q0.lerp(q1, t));
            return (p.distance(q) < near).then(|| Self::approach(x, y, p, q));
        }
        // Before any nearest point is sought: two edges proven further
        // apart than twice the tolerance everywhere give nothing below.
        if edges_apart(e1, e2, near, work) {
            return None;
        }
        // Along the edge whose nearest points are the dearer to find (a
        // spline), measured against the other.
        let flip = matches!(e2.curve, Curve3::Nurbs(_)) && !matches!(e1.curve, Curve3::Nurbs(_));
        let (along, other) = if flip { (e2, e1) } else { (e1, e2) };
        let distance = |pt: Vec3| {
            work.nearest(&other.curve, other.range, pt)
                .map_or(f64::INFINITY, |(_, d)| d)
        };
        let mut best: Option<Approach> = None;
        for t in least_along(&along.curve, along.range, near, distance) {
            if at_end(t, along.range) {
                continue;
            }
            let p = along.curve.eval(t);
            let Some((u, _)) = work.nearest(&other.curve, other.range, p) else {
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

    /// An edge against a face. A piece of a ball or a ring that the edge
    /// passes only where the rest of the ball would be is passed over
    /// before this ([`Look::away_from_piece`]).
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
            if self
                .work
                .surface_distance(Sheet::bare(&face.surface), middle)
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
        let work = &self.work;
        let distance = |pt: Vec3| {
            work.onto(sheet, pt)
                .map_or(f64::INFINITY, |q| pt.distance(q))
        };
        let mut best: Option<Approach> = None;
        for t in least_along(&edge.curve, r, near, distance) {
            if at_end(t, r) {
                continue;
            }
            let p = edge.curve.eval(t);
            let Some(q) = work.onto(sheet, p) else {
                continue;
            };
            let d = p.distance(q);
            if d < near && work.inside(o, q, tol) && best.is_none_or(|b| d < b.distance) {
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
        let far = |work: &Work, bx: &Bbox3, other: &Surface| {
            !is_freeform(other)
                && bx.corners().is_some_and(|(lo, hi)| {
                    work.surface_distance(Sheet::bare(other), lo.lerp(hi, 0.5))
                        > 0.5 * lo.distance(hi) + near
                })
        };
        let whole_f = body_x.face_bbox(FaceId(f)).ok()?;
        let whole_g = body_y.face_bbox(FaceId(g)).ok()?;
        if far(&self.work, &whole_f, &fg.surface) || far(&self.work, &whole_g, &ff.surface) {
            return None;
        }
        let box_f = self.topos.get_mut(x.0)?.held_box(f)?;
        let box_g = self.topos.get_mut(y.0)?.held_box(g)?;
        if !boxes_meet(&box_f, &box_g, near)
            || far(&self.work, &box_f, &fg.surface)
            || far(&self.work, &box_g, &ff.surface)
        {
            return None;
        }
        if box_f.corners().is_none() || box_g.corners().is_none() {
            return None;
        }
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
        let (p, q) = least_between(o1, o2, &box_f, &box_g, tol, near, &self.work)?;
        (p.distance(q) < near).then(|| Self::approach(x, y, p, q))
    }
}

/// Whether two ascending lists hold a value in common: the shorter looked
/// up in the longer.
fn share(a: &[u32], b: &[u32]) -> bool {
    let (few, many) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    few.iter().any(|v| many.binary_search(v).is_ok())
}

/// A stretch of an edge's curve, for [`edges_apart`]: every point of the
/// curve over `range` lies within `reach` of the segment from `a` to `b`
/// (of the point, where they are one).
#[derive(Clone, Copy, Debug)]
struct Stretch {
    range: (f64, f64),
    a: Vec3,
    b: Vec3,
    reach: f64,
}

impl Stretch {
    /// The stretch of `curve` over `range`, `speed` its speed bound
    /// ([`speed_bound`]): a line's piece is its chord. An arc of a circle
    /// of at most half a turn lies within its sagitta `2R sin²(φ/4)` of
    /// its chord (φ its angle), and an arc of an ellipse within that of the
    /// larger semi-axis (the ellipse is a circle's arc stretched, which
    /// stretches no distance by more than that axis). A piece of a spline
    /// lies in the convex hull of the control points of the spans it
    /// crosses (its weights positive), so within the farthest of them from
    /// its chord: that, where it is less than the bound by speed. Any other
    /// piece within `speed` times half its range of its middle point.
    fn of(curve: &Curve3, range: (f64, f64), speed: f64, work: &Work) -> Option<Stretch> {
        let (lo, hi) = range;
        if !(lo.is_finite() && hi.is_finite() && hi > lo) {
            return None;
        }
        let chord = |reach: f64| Stretch {
            range,
            a: work.eval(curve, lo),
            b: work.eval(curve, hi),
            reach,
        };
        let by_speed = || {
            let middle = work.eval(curve, 0.5 * (lo + hi));
            Stretch {
                range,
                a: middle,
                b: middle,
                reach: speed * 0.5 * (hi - lo),
            }
        };
        let quarter = (0.25 * (hi - lo)).psin();
        let stretch = match curve {
            Curve3::Line(_) => chord(0.0),
            Curve3::Circle(circle) if hi - lo <= PI => {
                chord(2.0 * circle.radius() * quarter * quarter)
            }
            Curve3::Ellipse(ellipse) if hi - lo <= PI => {
                let (a, b) = ellipse.semi_axes();
                chord(2.0 * a.max(b) * quarter * quarter)
            }
            Curve3::Nurbs(_) if past_domain(curve, range) => return None,
            Curve3::Nurbs(spline) => {
                let rho = speed * 0.5 * (hi - lo);
                match holding_points(spline, range) {
                    Some(points) => {
                        let hull = chord(0.0);
                        let reach = points
                            .iter()
                            .map(|&c| point_segment(c, hull.a, hull.b))
                            .fold(0.0_f64, f64::max);
                        if reach.is_finite() && reach < rho {
                            Stretch { reach, ..hull }
                        } else {
                            by_speed()
                        }
                    }
                    None => by_speed(),
                }
            }
            _ => by_speed(),
        };
        (stretch.a.is_finite()
            && stretch.b.is_finite()
            && stretch.reach.is_finite()
            && stretch.reach >= 0.0)
            .then_some(stretch)
    }

    /// Its two halves.
    fn halves(&self, curve: &Curve3, speed: f64, work: &Work) -> Option<(Stretch, Stretch)> {
        let (lo, hi) = self.range;
        let middle = 0.5 * (lo + hi);
        if !(lo < middle && middle < hi) {
            return None;
        }
        Some((
            Stretch::of(curve, (lo, middle), speed, work)?,
            Stretch::of(curve, (middle, hi), speed, work)?,
        ))
    }
}

/// The control points of a spline curve whose convex hull holds its piece
/// over `range`: those of the spans the range crosses (`P[a − p] ..=
/// P[b − 1]`, the range from the span that starts at knot `a` to the one
/// that ends at knot `b`, `p` the degree). None where the knots or the
/// weights (which must be positive for the hull to hold) are not as a
/// spline's.
fn holding_points(spline: &NurbsCurve, range: (f64, f64)) -> Option<&[Vec3]> {
    let p = spline.degree();
    let (knots, points, weights) = (spline.knots(), spline.control_points(), spline.weights());
    let n = points.len();
    if n == 0 || knots.len() != n + p + 1 || weights.len() != n {
        return None;
    }
    let (lo, hi) = range;
    let starts = knots.get(p..=n)?.partition_point(|&u| u <= lo);
    let a = (p + starts.saturating_sub(1)).min(n - 1);
    let b = (p + 1 + knots.get(p + 1..=n)?.partition_point(|&u| u < hi)).min(n);
    let (first, last) = (a.checked_sub(p)?, b.checked_sub(1)?);
    if first > last
        || !weights
            .get(first..=last)?
            .iter()
            .all(|&w| w.is_finite() && w > 0.0)
    {
        return None;
    }
    points.get(first..=last)
}

/// The distance from `p` to the segment from `s0` to `s1` (a point where
/// they are one).
fn point_segment(p: Vec3, s0: Vec3, s1: Vec3) -> f64 {
    let d = s1 - s0;
    let dd = d.dot(d);
    let t = if dd > 0.0 {
        ((p - s0).dot(d) / dd).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(s0 + d * t)
}

/// Whether a spline edge's range runs past its curve's domain (an edge may:
/// `NurbsCurve::eval` carries the end spans on as the polynomials or
/// rational functions they are, and `craft3d_topo`'s regions read such an
/// edge so, `region.rs` › `each_piece`). Past the domain the curve leaves
/// the convex hull of its control points, its speed bound does not hold,
/// and `craft3d_topo`'s box of the edge (its control points) does not hold
/// it either: no bound of the look reads such an edge (the review of round
/// three, 2026-11-25). A range that is not a number is past it too.
fn past_domain(curve: &Curve3, range: (f64, f64)) -> bool {
    match curve {
        Curve3::Nurbs(spline) => {
            let (lo, hi) = spline.domain();
            !(range.0 >= lo && range.1 <= hi)
        }
        Curve3::Line(_) | Curve3::Circle(_) | Curve3::Ellipse(_) => false,
    }
}

/// Whether two edges are proven further apart than `near` everywhere
/// (2026-11-25): their curves' stretches ([`Stretch`]) compared two by
/// two, a pair whose segments' least distance ([`segment_gap`], a bound
/// from below) less both reaches exceeds `near` passed over, the others
/// halved (the stretch with the larger reach) until every pair is passed
/// over; not proven where [`APART`] pairs were compared first, or a pair
/// of chords or points with no reach comes that near, or a stretch can be
/// halved no further, and never for a spline edge whose range runs past its
/// curve's domain ([`past_domain`]). Every point of each edge lies in a stretch, so where
/// this holds no two points of them, however found, are nearer than
/// `near`, and the search it spares would find no approach: passing the
/// pair over changes no answer. A margin of rounding (64 rounding errors
/// of the largest coordinate) on top.
fn edges_apart(e1: &Edge, e2: &Edge, near: f64, work: &Work) -> bool {
    let (c1, c2) = (&e1.curve, &e2.curve);
    if past_domain(c1, e1.range) || past_domain(c2, e2.range) {
        return false;
    }
    let (s1, s2) = (speed_bound(c1), speed_bound(c2));
    if !(s1.is_finite() && s2.is_finite() && s1 >= 0.0 && s2 >= 0.0) {
        return false;
    }
    let (Some(p), Some(q)) = (
        Stretch::of(c1, e1.range, s1, work),
        Stretch::of(c2, e2.range, s2, work),
    ) else {
        return false;
    };
    let largest = [p.a, p.b, q.a, q.b].into_iter().fold(0.0_f64, |m, v| {
        m.max(v.x.abs()).max(v.y.abs()).max(v.z.abs())
    });
    let limit = near + 64.0 * f64::EPSILON * (largest + p.reach + q.reach);
    if !limit.is_finite() {
        return false;
    }
    let mut stack = vec![(p, q)];
    let mut compared = 0usize;
    while let Some((p, q)) = stack.pop() {
        compared += 1;
        if compared > APART {
            return false;
        }
        let gap = segment_gap(p.a, p.b, q.a, q.b);
        if gap - p.reach - q.reach > limit {
            continue;
        }
        if p.reach <= 0.0 && q.reach <= 0.0 {
            return false;
        }
        if p.reach >= q.reach {
            let Some((p1, p2)) = p.halves(c1, s1, work) else {
                return false;
            };
            stack.push((p2, q));
            stack.push((p1, q));
        } else {
            let Some((q1, q2)) = q.halves(c2, s2, work) else {
                return false;
            };
            stack.push((p, q2));
            stack.push((p, q1));
        }
    }
    true
}

/// A bound from below of the distance between the segments `a0 a1` and
/// `b0 b1` (either may be a point), to rounding: the least of each end's
/// distance to the other segment and, unless the two are parallel, of the
/// distance between the lines through them less what rounding could add
/// to it. The distance is one of these (at an end of one segment, or
/// between the insides of both, where it is the lines'), and the lines'
/// distance is never more than the segments'. Zero for points that are
/// not finite.
fn segment_gap(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> f64 {
    if !(a0.is_finite() && a1.is_finite() && b0.is_finite() && b1.is_finite()) {
        return 0.0;
    }
    let to = point_segment;
    let mut gap = to(a0, b0, b1)
        .min(to(a1, b0, b1))
        .min(to(b0, a0, a1))
        .min(to(b1, a0, a1));
    let (d1, d2) = (a1 - a0, b1 - b0);
    let n = d1.cross(d2);
    let length = n.length();
    if length > 0.0 {
        let r = a0 - b0;
        let error = 16.0 * f64::EPSILON * r.length() * d1.length() * d2.length();
        let lines = (r.dot(n).abs() - error) / length;
        gap = gap.min(lines.max(0.0));
    }
    if gap.is_finite() { gap } else { 0.0 }
}

/// The nearest pair offered for which a test holds, nearer than a limit.
struct Least {
    limit: f64,
    best: Option<(Vec3, Vec3, f64)>,
}

impl Least {
    fn below(limit: f64) -> Least {
        Least { limit, best: None }
    }

    /// Kept where nearer than the limit and than the best so far (the first
    /// of equals stays), and `fits` holds.
    fn offer(&mut self, p: Vec3, q: Vec3, fits: impl Fn(Vec3, Vec3) -> bool) {
        let d = p.distance(q);
        if d < self.limit && self.best.is_none_or(|b| d < b.2) && fits(p, q) {
            self.best = Some((p, q, d));
        }
    }
}

/// The pair of points, one inside each face, nearest each other that the
/// search finds, if nearer than `limit` (see the module, *How near*);
/// `box_f` and `box_g` the boxes of what the faces hold.
///
/// - The search as it was before 2026-11-24's evening: alternating
///   projections from three seeds (the middles of the boxes and of their
///   common part), the nearest end inside both faces (within `limit`)
///   polished by golden sections across the first face ([`polish`]).
/// - Where the two surfaces' kinds have them in closed form
///   ([`square_pairs`]), the pairs square to both surfaces that lie inside
///   both faces: the least distance between points inside the faces and on
///   neither boundary is one of them, or lies along a line or a circle of
///   them, which either reaches a boundary (measured there as an edge or a
///   vertex) or lies wholly inside both faces (one pair of it is among
///   them).
/// - Otherwise from the end of every seed whose projections had not
///   settled in their steps, a descent ([`descend`]); from every end where
///   they settled (once for ends within the tolerance of each other),
///   which is a pair square to both surfaces and may be a saddle or a
///   greatest of the distance rather than its least, a way down where the
///   distance curves down across the first surface ([`escape`],
///   2026-11-25); and a cone's apex with its nearest point ([`apexes`]).
///
/// The least of all of these, the first of equals: so never further than
/// the search as it was. Every point lies on its surface, evaluated
/// exactly, and inside its face.
fn least_between(
    o1: &Outline,
    o2: &Outline,
    box_f: &Bbox3,
    box_g: &Bbox3,
    tol: f64,
    limit: f64,
    work: &Work,
) -> Option<(Vec3, Vec3)> {
    let ((lo1, hi1), (lo2, hi2)) = (box_f.corners()?, box_g.corners()?);
    let fits = |p: Vec3, q: Vec3| work.inside(o1, p, tol) && work.inside(o2, q, tol);
    work.add(1);
    let closed = square_pairs(&o1.surface, &o2.surface);
    let reach = 0.25 * box_f.diagonal().min(box_g.diagonal());
    let (common_lo, common_hi) = (lo1.max(lo2), hi1.min(hi2));
    let seeds = [
        common_lo.lerp(common_hi, 0.5),
        lo1.lerp(hi1, 0.5),
        lo2.lerp(hi2, 0.5),
    ];
    let mut ends = Least::below(limit);
    let mut descents: Vec<(Vec3, Vec3)> = Vec::new();
    let mut settled_at: Vec<Vec3> = Vec::new();
    for seed in seeds {
        if !seed.is_finite() {
            continue;
        }
        let Some((p, q, settled)) = alternate(o1, o2, seed, tol, work) else {
            continue;
        };
        ends.offer(p, q, fits);
        if closed.is_some() {
            continue;
        }
        if !settled {
            descents.extend(descend(o1, o2, p, reach, tol, work, fits));
        } else if !settled_at.iter().any(|&at| at.distance(p) <= tol) {
            settled_at.push(p);
            descents.extend(escape(o1, o2, p, reach, tol, work, fits));
        }
    }
    let mut least = Least::below(limit);
    // Where two faces touch tangentially the projections crawl towards the
    // place of touch and stop short of it: across the first face from the
    // nearest end, to the least distance there.
    if let Some((p, q, d)) = ends.best {
        let (p2, q2) = polish(o1, o2, p, q, reach, work);
        if p2.distance(q2) < d && fits(p2, q2) {
            least.offer(p2, q2, fits);
        } else {
            least.offer(p, q, fits);
        }
    }
    for (p, q) in descents {
        least.offer(p, q, fits);
    }
    match closed {
        Some(pairs) => {
            for (p, q) in pairs {
                least.offer(p, q, fits);
            }
        }
        None => {
            for (p, q) in apexes(o1, o2, work) {
                least.offer(p, q, fits);
            }
        }
    }
    least.best.map(|(p, q, _)| (p, q))
}

/// Alternating projections between the two faces' surfaces from `seed`,
/// at most [`ALTERNATE`] steps: the last pair (a point of the first
/// surface, and the point of the second it was projected from), and
/// whether they settled (the last step moved no more than a millionth of
/// the tolerance). None where no step could be taken.
fn alternate(
    o1: &Outline,
    o2: &Outline,
    seed: Vec3,
    tol: f64,
    work: &Work,
) -> Option<(Vec3, Vec3, bool)> {
    let mut p = work.onto(o1.sheet(), seed)?;
    let mut q = p;
    let (mut moved_any, mut settled) = (false, false);
    for _ in 0..ALTERNATE {
        let Some(q2) = work.onto(o2.sheet(), p) else {
            break;
        };
        let Some(p2) = work.onto(o1.sheet(), q2) else {
            break;
        };
        let moved = p2.distance(p);
        p = p2;
        q = q2;
        moved_any = true;
        if moved <= 1e-6 * tol {
            settled = true;
            break;
        }
    }
    moved_any.then_some((p, q, settled))
}

/// The pair of a point `x` projected onto the first face's surface and
/// that point projected onto the second's, and their distance: what
/// [`descend`], [`escape`] and [`polish`] move over the first surface.
fn pair_at(o1: &Outline, o2: &Outline, x: Vec3, work: &Work) -> Option<(Vec3, Vec3, f64)> {
    let a = work.onto(o1.sheet(), x)?;
    let b = work.onto(o2.sheet(), a)?;
    let d = a.distance(b);
    d.is_finite().then_some((a, b, d))
}

/// A descent of the distance from the first face's surface to the
/// second's, from `start` on the first: each step a golden-section line
/// search ([`golden`], at most `reach` far) along the part of `q − p` in
/// the first surface's tangent plane at `p` (`q` the point of the second
/// surface nearest `p`: the direction in which the distance falls
/// fastest), first bent by the last direction (conjugate directions,
/// Fletcher and Reeves: two steps reach the least of a quadratic over a
/// plane), taken only where the distance falls. At most [`DESCENT`] steps,
/// ending where one gains no more than a millionth of the tolerance.
///
/// Alternating projections step by about `gap · sin θ` (θ the angle
/// between the normals at the two points), so between faces nearly
/// tangent and a little apart they crawl towards the nearest pair (a ball
/// of radius 20 a tolerance and a half over a plane, by 0.04 % of the way a
/// step: after forty steps still 0.22 units off, the second review's
/// dimple); a line search takes the step the distance asks for. Every pair
/// it meets is a point of each surface, evaluated exactly: the nearest of
/// them for which `fits` holds (the start's too).
fn descend(
    o1: &Outline,
    o2: &Outline,
    start: Vec3,
    reach: f64,
    tol: f64,
    work: &Work,
    fits: impl Fn(Vec3, Vec3) -> bool,
) -> Option<(Vec3, Vec3)> {
    let at = |x: Vec3| pair_at(o1, o2, x, work);
    let mut now = at(start)?;
    let mut kept = fits(now.0, now.1).then_some(now);
    if !(reach.is_finite() && reach > 0.0) {
        return kept.map(|(p, q, _)| (p, q));
    }
    let mut last: Option<(Vec3, f64)> = None;
    for _ in 0..DESCENT {
        let (p, q, d) = now;
        let Some(normal) = work.normal(o1.sheet(), p) else {
            break;
        };
        let pull = q - p;
        let down = pull - normal * pull.dot(normal);
        let slope = down.length_squared();
        if !(slope.is_finite() && slope > 0.0) {
            break;
        }
        let bent = last.map(|(dir, before)| {
            let carried = dir - normal * dir.dot(normal);
            down + carried * (slope / before)
        });
        let mut step = None;
        for dir in bent.into_iter().chain([down]) {
            let Some(unit) = dir.normalized() else {
                continue;
            };
            let s = golden(0.0, reach, |s| {
                at(p + unit * s).map_or(f64::INFINITY, |found| found.2)
            });
            if let Some(found) = at(p + unit * s)
                && found.2 < d
            {
                step = Some((found, dir));
                break;
            }
        }
        let Some((found, dir)) = step else {
            break;
        };
        now = found;
        last = Some((dir, slope));
        if fits(found.0, found.1) && kept.is_none_or(|k| found.2 < k.2) {
            kept = Some(found);
        }
        if d - found.2 <= 1e-6 * tol {
            break;
        }
    }
    kept.map(|(p, q, _)| (p, q))
}

/// Where the alternating projections settled at `p` on the first face's
/// surface (2026-11-25): the pair there is square to both surfaces, so
/// the distance is stationary at it, but it may be a saddle (least across
/// a ring slid on a rod off its axis, greatest round it: the review of the
/// dimple and the caps, F2) or a greatest, where no descent starts (the
/// slope is nothing). Its curvature across the first surface, from second
/// differences of the distance [`ESCAPE`] times `reach` either way along
/// two directions square to the normal and along both diagonals: where it
/// curves down along some direction by more than rounding (and a
/// millionth of the tolerance), a golden-section line search along the
/// direction it curves down most, `reach` either way, and a descent
/// ([`descend`]) from the pair it reaches where that is nearer. None
/// where the distance curves up every way (a least, which the
/// projections' end already offers) or no step could be taken; otherwise
/// what the descent keeps.
fn escape(
    o1: &Outline,
    o2: &Outline,
    p: Vec3,
    reach: f64,
    tol: f64,
    work: &Work,
    fits: impl Fn(Vec3, Vec3) -> bool,
) -> Option<(Vec3, Vec3)> {
    if !(reach.is_finite() && reach > 0.0) {
        return None;
    }
    let at = |x: Vec3| pair_at(o1, o2, x, work);
    let (p, q, d) = at(p)?;
    let normal = work.normal(o1.sheet(), p)?;
    let aside = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let e1 = normal.cross(aside).normalized()?;
    let e2 = normal.cross(e1);
    let h = ESCAPE * reach;
    let f = |s: f64, t: f64| at(p + e1 * s + e2 * t).map_or(f64::NAN, |found| found.2);
    let a = f(h, 0.0) + f(-h, 0.0) - 2.0 * d;
    let c = f(0.0, h) + f(0.0, -h) - 2.0 * d;
    let b = 0.25 * (f(h, h) - f(h, -h) - f(-h, h) + f(-h, -h));
    if !(a.is_finite() && b.is_finite() && c.is_finite()) {
        return None;
    }
    // The least of the two curvatures (times h²) and its direction.
    let least = 0.5 * (a + c) - (0.5 * (a - c)).phypot(b);
    let largest = [p, q].into_iter().fold(0.0_f64, |m, v| {
        m.max(v.x.abs()).max(v.y.abs()).max(v.z.abs())
    });
    let floor = 1e-6 * tol + 64.0 * f64::EPSILON * (largest + d);
    if least.is_nan() || least >= -floor {
        return None;
    }
    let (one, two) = ((b, least - a), (least - c, b));
    let (u, v) = if one.0.phypot(one.1) >= two.0.phypot(two.1) {
        one
    } else {
        two
    };
    let dir = (e1 * u + e2 * v).normalized()?;
    let s = golden(-reach, reach, |s| {
        at(p + dir * s).map_or(f64::INFINITY, |found| found.2)
    });
    let found = at(p + dir * s)?;
    if found.2.is_nan() || found.2 >= d {
        return None;
    }
    descend(o1, o2, found.0, reach, tol, work, fits)
}

/// The apex of a cone (where its surface has no normal, so no pair of
/// [`square_pairs`] and no settled projection need be there) with the
/// point of the other face's surface nearest it, each way; both apexes
/// where both are cones.
fn apexes(o1: &Outline, o2: &Outline, work: &Work) -> Vec<(Vec3, Vec3)> {
    let (a1, a2) = (apex(&o1.surface), apex(&o2.surface));
    let mut out = Vec::new();
    if let Some(a) = a1
        && let Some(b) = work.onto(o2.sheet(), a)
    {
        out.push((a, b));
    }
    if let Some(b) = a2
        && let Some(a) = work.onto(o1.sheet(), b)
    {
        out.push((a, b));
    }
    if let (Some(a), Some(b)) = (a1, a2) {
        out.push((a, b));
    }
    out
}

/// The apex of a cone.
fn apex(surface: &Surface) -> Option<Vec3> {
    match surface {
        Surface::Cone(cone) => Some(apex_of(cone)).filter(|p| p.is_finite()),
        _ => None,
    }
}

fn apex_of(cone: &Cone) -> Vec3 {
    cone.frame().point(0.0, 0.0, cone.apex_v())
}

/// The pairs of points, one of each surface (`(on s1, on s2)`), on a line
/// square to both: where the distance between the two surfaces is
/// stationary, so the nearest pair of two faces on them, unless it lies on
/// a boundary, is one of these. In closed form for a plane or a ball
/// against any analytic surface, and for two cylinders: every such pair
/// that is alone, and one pair of each circle of them (a ring lying flat
/// over a plane, a ball on a pipe's axis or a ring's, two balls about one
/// centre; any pair of a circle stands for it, since the faces either hold
/// all of it or a boundary crosses it). Lines of them (a rod lying over a
/// plane, two parallel rods, a cone's generator over a plane) are left
/// out: a face ends somewhere along each, and a boundary that crosses one
/// is looked at as an edge or a vertex. A cone's apex is among the points
/// (the surface has no normal there). `None` where the kinds have no
/// closed form: two of a cylinder, a cone and a torus but two cylinders,
/// and a freeform surface.
fn square_pairs(s1: &Surface, s2: &Surface) -> Option<Vec<(Vec3, Vec3)>> {
    let swap = |pairs: Vec<(Vec3, Vec3)>| -> Vec<(Vec3, Vec3)> {
        pairs.into_iter().map(|(p, q)| (q, p)).collect()
    };
    let pairs = match (s1, s2) {
        (Surface::Plane(plane), other) => from_plane(plane, other)?,
        (other, Surface::Plane(plane)) => swap(from_plane(plane, other)?),
        (Surface::Sphere(ball), other) => from_ball(ball, other)?,
        (other, Surface::Sphere(ball)) => swap(from_ball(ball, other)?),
        (Surface::Cylinder(c1), Surface::Cylinder(c2)) => two_cylinders(c1, c2),
        _ => return None,
    };
    Some(
        pairs
            .into_iter()
            .filter(|(p, q)| p.is_finite() && q.is_finite())
            .collect(),
    )
}

/// [`square_pairs`] of a plane and another surface, `(on the plane, on the
/// other)`: the points of the other where its normal is the plane's (where
/// the height over the plane is stationary), each with its foot.
fn from_plane(plane: &Plane, other: &Surface) -> Option<Vec<(Vec3, Vec3)>> {
    let frame = plane.frame();
    let (origin, n) = (frame.origin(), frame.z());
    let points: Vec<Vec3> = match other {
        // Two planes: parallel (a plane of pairs) or none. A cylinder: the
        // lines along it whose normal is the plane's, where its axis is
        // parallel to the plane, or none.
        Surface::Plane(_) | Surface::Cylinder(_) => Vec::new(),
        // Lines along the cone, and its apex.
        Surface::Cone(cone) => vec![apex_of(cone)],
        Surface::Sphere(ball) => {
            let (c, r) = (ball.frame().origin(), ball.radius());
            vec![c - n * r, c + n * r]
        }
        Surface::Torus(torus) => {
            // The torus's point at (u, v) is `o + R w(u) + r N(u, v)`, its
            // normal `N = cos v w(u) + sin v z`: where `N = ±n`, `w(u)` is
            // `±` the part of `n` square to the axis. Where `n` is along
            // the axis, the circles on top and underneath: one point of
            // each. The part square to the axis taken twice, so that
            // rounding leaves it square to the axis. Along the axis means
            // to the rounding of that part (a ring lying flat leaves about
            // 1e-16 of it), not to an angle: the direction of a part `m`
            // long is off by about `ε / |m|`, which moves the pairs' height
            // over the plane by about `R ε` whatever `m` is, while a ring
            // leaning `|m|` rad taken as flat puts its pairs a quarter turn
            // from its lowest point, `R |m|` higher (the review of the
            // dimple and the caps, F1: 1e-9 here took a ring of radius 3000
            // leaning 8e-10 rad as flat, 2.4 tolerances off).
            let f = torus.frame();
            let (major, minor) = torus.radii();
            let axis = f.z();
            let m = n - axis * n.dot(axis);
            let m = m - axis * m.dot(axis);
            let (across, up) = match m.normalized() {
                Some(across) if m.length() > 1e-12 => (across, n),
                _ => (f.x(), axis),
            };
            let mut out = Vec::with_capacity(4);
            for side in [1.0, -1.0] {
                for sign in [1.0, -1.0] {
                    out.push(f.origin() + across * (side * major) + up * (sign * minor));
                }
            }
            out
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    Some(
        points
            .into_iter()
            .map(|x| (x - n * (x - origin).dot(n), x))
            .collect(),
    )
}

/// [`square_pairs`] of a ball and another surface, `(on the ball, on the
/// other)`: the points of the other whose normal passes through the
/// ball's centre ([`normals_through`]), each with the two points of the
/// ball on that line.
fn from_ball(ball: &Sphere, other: &Surface) -> Option<Vec<(Vec3, Vec3)>> {
    let (c, r) = (ball.frame().origin(), ball.radius());
    let feet = normals_through(other, c)?;
    let mut out = Vec::with_capacity(2 * feet.len());
    for x in feet {
        let along = match (x - c).normalized() {
            Some(along) => along,
            // The centre on the other surface: along its normal there.
            None => match other.project(x).ok().and_then(|(u, v)| other.normal(u, v)) {
                Some(normal) => normal,
                None => continue,
            },
        };
        out.push((c + along * r, x));
        out.push((c - along * r, x));
    }
    Some(out)
}

/// The points of an analytic surface whose normal passes through `c`
/// (where the distance from `c` is stationary), in closed form, with one
/// point of each circle of them; a cone's apex among them. `None` for a
/// freeform surface.
fn normals_through(surface: &Surface, c: Vec3) -> Option<Vec<Vec3>> {
    Some(match surface {
        Surface::Plane(plane) => {
            let f = plane.frame();
            vec![c - f.z() * (c - f.origin()).dot(f.z())]
        }
        Surface::Sphere(ball) => {
            let f = ball.frame();
            let (o, r) = (f.origin(), ball.radius());
            // About one centre, every point: one of them.
            let along = (c - o).normalized().unwrap_or(f.x());
            vec![o + along * r, o - along * r]
        }
        Surface::Cylinder(cylinder) => {
            let f = cylinder.frame();
            let axis = f.z();
            let foot = f.origin() + axis * (c - f.origin()).dot(axis);
            // Square to the axis whatever rounding left of `c − foot`; on
            // the axis, the circle about it: one point of it.
            let off = c - foot;
            let out = (off - axis * off.dot(axis)).normalized().unwrap_or(f.x());
            let r = cylinder.radius();
            vec![foot + out * r, foot - out * r]
        }
        Surface::Cone(cone) => {
            // In the half-plane through the axis and `c` the cone is a ray
            // from the apex (and in the opposite half-plane another): the
            // foot of `c` on each, where it lies beyond the apex.
            let f = cone.frame();
            let (lx, ly, lz) = f.local(c);
            let rho = lx.phypot(ly);
            let (ex, ey) = if rho > 0.0 {
                (lx / rho, ly / rho)
            } else {
                (1.0, 0.0)
            };
            let tip = cone.apex_v();
            let sigma = if cone.half_angle() > 0.0 { 1.0 } else { -1.0 };
            let (sine, cosine) = cone.half_angle().abs().psin_cos();
            let mut out = vec![apex_of(cone)];
            for side in [1.0, -1.0] {
                let s = side * rho * sine + (lz - tip) * sigma * cosine;
                if s.is_finite() && s > 0.0 {
                    let radial = side * s * sine;
                    out.push(f.point(radial * ex, radial * ey, tip + s * sigma * cosine));
                }
            }
            out
        }
        Surface::Torus(torus) => {
            // The normal at a point passes through the middle of the tube
            // there, in the half-plane through the axis: in the half-plane
            // through `c` and in the opposite one, the two points of the
            // tube's circle on the line from its middle to `c`.
            // Worked out in the half-plane's own terms (along `e` and the
            // axis), so that the points lie on the tube's circle there.
            let f = torus.frame();
            let (major, minor) = torus.radii();
            let (lx, ly, lz) = f.local(c);
            let rho = lx.phypot(ly);
            let (ex, ey) = if rho > 0.0 {
                (lx / rho, ly / rho)
            } else {
                (1.0, 0.0)
            };
            let mut out = Vec::with_capacity(4);
            for side in [1.0, -1.0] {
                let middle = f.point(side * major * ex, side * major * ey, 0.0);
                let (radial, up) = (rho - side * major, lz);
                let length = radial.phypot(up);
                let along = if length > 0.0 {
                    f.vector(radial / length * ex, radial / length * ey, up / length)
                } else {
                    f.vector(side * ex, side * ey, 0.0)
                };
                out.push(middle + along * minor);
                out.push(middle - along * minor);
            }
            out
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    })
}

/// [`square_pairs`] of two cylinders: the line square to both axes (where
/// they are not parallel), and on it the two points of each cylinder. Two
/// parallel axes leave lines of pairs, or every pair about one axis.
fn two_cylinders(c1: &Cylinder, c2: &Cylinder) -> Vec<(Vec3, Vec3)> {
    let (f1, f2) = (c1.frame(), c2.frame());
    let (o1, d1, o2, d2) = (f1.origin(), f1.z(), f2.origin(), f2.z());
    let across = d1.cross(d2);
    let sin2 = across.length_squared();
    let Some(dir) = across.normalized().filter(|_| sin2 > 1e-24) else {
        return Vec::new();
    };
    // The feet of the common perpendicular on each axis.
    let w = o1 - o2;
    let (b, d, e) = (d1.dot(d2), d1.dot(w), d2.dot(w));
    let (s, t) = ((b * e - d) / sin2, (e - b * d) / sin2);
    let (foot1, foot2) = (o1 + d1 * s, o2 + d2 * t);
    let (r1, r2) = (c1.radius(), c2.radius());
    let mut out = Vec::with_capacity(4);
    for one in [1.0, -1.0] {
        for two in [1.0, -1.0] {
            out.push((foot1 + dir * (one * r1), foot2 + dir * (two * r2)));
        }
    }
    out
}

/// The ids both ascending lists hold.
fn common(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while let (Some(&x), Some(&y)) = (a.get(i), b.get(j)) {
        match x.cmp(&y) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(x);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// The points of two faces' surfaces nearest each other about `p` on the
/// first (and `q` on the second): golden sections along two directions
/// across the first surface at `p`, `reach` either way, twice over, each
/// keeping only a pair nearer than the last.
fn polish(o1: &Outline, o2: &Outline, p: Vec3, q: Vec3, reach: f64, work: &Work) -> (Vec3, Vec3) {
    let mut best = (p, q, p.distance(q));
    if !(reach.is_finite() && reach > 0.0) {
        return (p, q);
    }
    let Some(normal) = work.normal(o1.sheet(), p) else {
        return (p, q);
    };
    let aside = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let Some(e1) = normal.cross(aside).normalized() else {
        return (p, q);
    };
    let e2 = normal.cross(e1);
    let at = |from: Vec3, dir: Vec3, s: f64| pair_at(o1, o2, from + dir * s, work);
    for _ in 0..2 {
        for dir in [e1, e2] {
            let from = best.0;
            let s = golden(-reach, reach, |s| {
                at(from, dir, s).map_or(f64::INFINITY, |found| found.2)
            });
            if let Some(found) = at(from, dir, s)
                && found.2 < best.2
            {
                best = found;
            }
        }
    }
    (best.0, best.1)
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

#[cfg(test)]
mod look_3_review;

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::KernelError;
    use crate::boolean::hits::signed_distance;
    use crate::boolean::{BooleanOp, Interference, boolean, interference};
    use crate::primitives;
    use craft3d_geom::{Circle, Ellipse, Line, NurbsSurface, Torus};
    use craft3d_math::{Frame, Tolerance};
    use craft3d_topo::{BodyKind, Coedge, VertexId};
    use std::cell::Cell;

    thread_local! {
        /// Caps a test of this thread set (`(work, approaches)`), which
        /// [`super::caps`] gives instead of the real ones.
        pub(super) static CAPS: Cell<Option<(u64, usize)>> = const { Cell::new(None) };
    }

    const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

    /// The world's frame, and a frame turned in space at scale `k` (the
    /// near-approach tests').
    fn frames(k: f64) -> [Frame; 2] {
        [
            Frame::WORLD,
            Frame::new(
                Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
                Vec3::new(0.3, -0.5, 0.81),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap(),
        ]
    }

    fn normal_at(surface: &Surface, p: Vec3) -> Option<Vec3> {
        surface
            .project(p)
            .ok()
            .and_then(|(u, v)| surface.normal(u, v))
    }

    /// The pairs square to two analytic surfaces in closed form, for every
    /// pair of kinds that has them, a circle of them included (a ring lying
    /// flat over a plane, a ball on a pipe's axis, two balls about one
    /// centre): each point lies on its surface, each pair (but at a cone's
    /// apex, which has no normal) on a line square to both, and the least
    /// of their distances is the least distance between the two surfaces
    /// as worked out here by other means (heights, distances to an axis or
    /// a circle, the common perpendicular of two lines), either way round.
    /// A plane and a cylinder have none (lines of them, or none). At 1e-3,
    /// 1 and 1e3, as found and turned.
    #[test]
    fn the_pairs_square_to_two_analytic_surfaces_hold_their_least_distance() {
        let (s30, c30) = 30.0_f64.to_radians().psin_cos();
        let (s20, c20) = 20.0_f64.to_radians().psin_cos();
        let (s03, c03) = 0.3_f64.psin_cos();
        for k in SCALES {
            for frame in frames(k) {
                let at = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
                let placed = |x: f64, y: f64, z: f64| frame.at(at(x, y, z)).unwrap();
                let along = |x: f64, y: f64, z: f64, axis: Vec3, hint: Vec3| {
                    Frame::new(
                        at(x, y, z),
                        frame.vector(axis.x, axis.y, axis.z),
                        frame.vector(hint.x, hint.y, hint.z),
                    )
                    .unwrap()
                };
                let plane = Surface::Plane(Plane::new(frame));
                let ball = |x: f64, y: f64, z: f64, r: f64| {
                    Surface::Sphere(Sphere::new(placed(x, y, z), r * k).unwrap())
                };
                let lowest_apex = 4.0 - c20 / 0.6_f64.ptan();
                let cases: Vec<(&str, Surface, Surface, f64)> = vec![
                    (
                        "a ball over a plane",
                        plane.clone(),
                        ball(2.0, 3.0, 4.0, 3.5),
                        0.5,
                    ),
                    (
                        "a ring tilted 30 degrees over a plane",
                        plane.clone(),
                        Surface::Torus(
                            Torus::new(
                                along(1.0, -2.0, 5.0, Vec3::new(0.0, -s30, c30), Vec3::X),
                                3.0 * k,
                                k,
                            )
                            .unwrap(),
                        ),
                        5.0 - 3.0 * s30 - 1.0,
                    ),
                    (
                        "a ring lying flat over a plane",
                        plane.clone(),
                        Surface::Torus(
                            Torus::new(placed(0.0, 0.0, 3.0), 2.0 * k, 0.5 * k).unwrap(),
                        ),
                        2.5,
                    ),
                    (
                        "a cone's apex over a plane",
                        plane.clone(),
                        Surface::Cone(
                            Cone::new(
                                along(1.0, 1.0, 4.0, Vec3::new(0.0, -s20, c20), Vec3::X),
                                k,
                                0.6,
                            )
                            .unwrap(),
                        ),
                        lowest_apex,
                    ),
                    (
                        "two balls",
                        ball(0.0, 0.0, 0.0, 2.0),
                        ball(3.0, 4.0, 1.0, 2.5),
                        26.0_f64.sqrt() - 4.5,
                    ),
                    (
                        "two balls about one centre",
                        ball(1.0, 2.0, 3.0, 3.0),
                        ball(1.0, 2.0, 3.0, 2.2),
                        0.8,
                    ),
                    (
                        "a ball beside a rod",
                        Surface::Cylinder(
                            Cylinder::new(
                                along(0.0, 0.0, 0.0, Vec3::new(1.0, 1.0, 0.0), Vec3::Z),
                                1.5 * k,
                            )
                            .unwrap(),
                        ),
                        ball(2.0, -1.0, 3.0, 1.0),
                        13.5_f64.sqrt() - 2.5,
                    ),
                    (
                        "a ball on a pipe's axis",
                        Surface::Cylinder(Cylinder::new(placed(0.0, 0.0, 0.0), 3.0 * k).unwrap()),
                        ball(0.0, 0.0, 2.0, 2.5),
                        0.5,
                    ),
                    (
                        "a ball beside a cone",
                        Surface::Cone(Cone::new(placed(0.0, 0.0, 0.0), 2.0 * k, 0.3).unwrap()),
                        ball(6.0, 1.0, 3.0, 1.0),
                        (37.0_f64.sqrt() - 2.0) * c03 - 3.0 * s03 - 1.0,
                    ),
                    (
                        "a ball beside a ring",
                        Surface::Torus(Torus::new(placed(0.0, 0.0, 0.0), 4.0 * k, k).unwrap()),
                        ball(7.0, 2.0, 1.5, 1.0),
                        (53.0_f64.sqrt() - 4.0).phypot(1.5) - 2.0,
                    ),
                    (
                        "two rods across",
                        Surface::Cylinder(
                            Cylinder::new(along(0.0, 0.0, 0.0, Vec3::X, Vec3::Y), k).unwrap(),
                        ),
                        Surface::Cylinder(
                            Cylinder::new(
                                along(0.0, 0.0, 3.0, Vec3::new(0.6, 0.8, 0.0), Vec3::Z),
                                0.8 * k,
                            )
                            .unwrap(),
                        ),
                        1.2,
                    ),
                ];
                for (what, s1, s2, want) in &cases {
                    for (a, b, way) in [(s1, s2, "as given"), (s2, s1, "the other way round")] {
                        let pairs =
                            square_pairs(a, b).unwrap_or_else(|| panic!("{what}: no closed form"));
                        assert!(!pairs.is_empty(), "{what}: no pairs");
                        let mut least = f64::INFINITY;
                        for &(p, q) in &pairs {
                            for (s, x) in [(a, p), (b, q)] {
                                let off = signed_distance(Sheet::bare(s), x).unwrap();
                                assert!(
                                    off.abs() <= 1e-8 * k,
                                    "{what} at {k}, {way}: {x:?} lies {off} off its surface"
                                );
                            }
                            let d = p.distance(q);
                            least = least.min(d);
                            if d > 1e-6 * k {
                                let line = (p - q) * (1.0 / d);
                                for (s, x) in [(a, p), (b, q)] {
                                    if let Some(n) = normal_at(s, x) {
                                        assert!(
                                            line.cross(n).length() <= 1e-7,
                                            "{what} at {k}, {way}: not square to the surface at {x:?}"
                                        );
                                    }
                                }
                            }
                        }
                        assert!(
                            (least - want * k).abs() <= 1e-8 * k,
                            "{what} at {k}, {way}: the least pair {least}, the least distance {}",
                            want * k
                        );
                    }
                }
                let rod = Surface::Cylinder(
                    Cylinder::new(along(0.0, 0.0, 2.0, Vec3::X, Vec3::Y), k).unwrap(),
                );
                assert_eq!(
                    square_pairs(&plane, &rod),
                    Some(Vec::new()),
                    "a rod over a plane"
                );
            }
        }
    }

    /// A bicubic spline sheet over `[−half, half]²` in `frame` at height
    /// `lift + ((x − low.0)² + (y − low.1)²) / (2 radius)` (a paraboloid,
    /// which a bicubic holds exactly; flat where `radius` is infinite), its
    /// lowest point `lift` at `low`, bounded by its four border curves: one
    /// face, its normal along the frame's z.
    fn spline_sheet(frame: &Frame, half: f64, lift: f64, radius: f64, low: (f64, f64)) -> Body {
        // The cubic Bernstein coefficients of `(x − c)² / (2 radius)` along
        // `x = −half + 2 half u`: its quadratic's, raised a degree.
        let bowl = |c: f64| -> [f64; 4] {
            if !radius.is_finite() {
                return [0.0; 4];
            }
            let (e, l) = (-half - c, 2.0 * half);
            let (q0, q1, q2) = (
                e * e / (2.0 * radius),
                e * l / radius,
                l * l / (2.0 * radius),
            );
            let (b0, b1, b2) = (q0, q0 + 0.5 * q1, q0 + q1 + q2);
            [b0, (b0 + 2.0 * b1) / 3.0, (2.0 * b1 + b2) / 3.0, b2]
        };
        let (bx, by) = (bowl(low.0), bowl(low.1));
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, point) in row.iter_mut().enumerate() {
                *point = frame.point(
                    -half + 2.0 * half * i as f64 / 3.0,
                    -half + 2.0 * half * j as f64 / 3.0,
                    lift + bx[i] + by[j],
                );
            }
        }
        let mut body = Body::new(BodyKind::Sheet);
        let corners = [(0usize, 0usize), (3, 0), (3, 3), (0, 3)];
        let v: Vec<_> = corners
            .iter()
            .map(|&(i, j)| body.add_vertex(net[i][j]))
            .collect();
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let cubic = |points: Vec<Vec3>| {
            Curve3::Nurbs(NurbsCurve::polynomial(3, knots.clone(), points).unwrap())
        };
        let e0 = body.add_edge(
            cubic((0..4).map(|i| net[i][0]).collect()),
            (0.0, 1.0),
            v[0],
            v[1],
        );
        let e1 = body.add_edge(
            cubic((0..4).map(|j| net[3][j]).collect()),
            (0.0, 1.0),
            v[1],
            v[2],
        );
        let e2 = body.add_edge(
            cubic((0..4).map(|i| net[i][3]).collect()),
            (0.0, 1.0),
            v[3],
            v[2],
        );
        let e3 = body.add_edge(
            cubic((0..4).map(|j| net[0][j]).collect()),
            (0.0, 1.0),
            v[0],
            v[3],
        );
        let coedge = |edge, reversed| Coedge { edge, reversed };
        let rim = body.add_loop(vec![
            coedge(e0, false),
            coedge(e1, false),
            coedge(e2, true),
            coedge(e3, true),
        ]);
        let ctrl: Vec<Vec3> = net.iter().flatten().copied().collect();
        let surface =
            NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, vec![1.0; 16]).unwrap();
        body.add_face(Surface::Nurbs(surface), false, vec![rim]);
        body
    }

    /// The face of `body` on the plane through `at` square to `normal`.
    fn face_on(body: &Body, at: Vec3, normal: Vec3, k: f64) -> usize {
        body.faces
            .iter()
            .position(|face| match &face.surface {
                Surface::Plane(plane) => {
                    let f = *plane.frame();
                    f.z().cross(normal).length() <= 1e-9
                        && (at - f.origin()).dot(f.z()).abs() <= 1e-9 * k
                }
                _ => false,
            })
            .unwrap()
    }

    /// The least of `distance` over a spline surface's points, sampled on
    /// a grid of 101 by 101 of its parameters, then on one as fine about
    /// the least sample, 1/25 of the domain across.
    fn sampled(surface: &Surface, distance: impl Fn(Vec3) -> f64) -> f64 {
        let grid = |u0: f64, u1: f64, v0: f64, v1: f64, best: &mut (f64, f64, f64)| {
            for i in 0..=100 {
                for j in 0..=100 {
                    let u = u0 + (u1 - u0) * f64::from(i) / 100.0;
                    let v = v0 + (v1 - v0) * f64::from(j) / 100.0;
                    let d = distance(surface.eval(u, v));
                    if d < best.0 {
                        *best = (d, u, v);
                    }
                }
            }
        };
        let mut best = (f64::INFINITY, 0.5, 0.5);
        grid(0.0, 1.0, 0.0, 1.0, &mut best);
        let (u, v) = (best.1, best.2);
        grid(
            (u - 0.02).max(0.0),
            (u + 0.02).min(1.0),
            (v - 0.02).max(0.0),
            (v + 0.02).min(1.0),
            &mut best,
        );
        best.0
    }

    /// The least distance between two faces that [`least_between`] finds
    /// (each with the box of what it holds), either way round (the same).
    fn least_of(x: (&Body, usize), y: (&Body, usize), tol: f64) -> f64 {
        let outline = |(body, f): (&Body, usize)| section::face_outline(body, f, tol).unwrap();
        let held = |(body, f): (&Body, usize)| body.held_face_bbox(FaceId(index(f))).unwrap();
        let (o1, o2) = (outline(x), outline(y));
        let (b1, b2) = (held(x), held(y));
        let work = Work::new();
        let one = least_between(&o1, &o2, &b1, &b2, tol, f64::INFINITY, &work)
            .map_or(f64::INFINITY, |(p, q)| p.distance(q));
        let other = least_between(&o2, &o1, &b2, &b1, tol, f64::INFINITY, &work)
            .map_or(f64::INFINITY, |(p, q)| p.distance(q));
        assert!(
            (one - other).abs() <= 1e-6 * tol,
            "either way round: {one} and {other}"
        );
        one
    }

    /// The faces of a spline sheet and a plane or a ball, nearest at one
    /// point inside both 1.5 tolerances apart, where they are nearly
    /// tangent, so that alternating projections crawl (the second review's
    /// dimple, with a spline face): a paraboloid of radius 20 or 5 over a
    /// plate's top, its lowest point off the middle, and a ball of radius
    /// 20 or 5 over a flat spline sheet, both ways round. The least
    /// distance found is the least there is (1.5 tolerances, by
    /// construction) within a ten-thousandth of the tolerance, and no
    /// further than the least of a dense sampling of the spline surface
    /// (the other surface's distance exact: a height, or the distance from
    /// the centre less the radius), which is itself within a thousandth of
    /// a tolerance of it. The plate and the ball, which are in closed form,
    /// exactly. At 1e-3, 1 and 1e3, as found and turned.
    #[test]
    fn the_least_between_a_spline_face_and_a_plane_or_a_ball_is_found() {
        for k in SCALES {
            let tol = 0.01 * k;
            let gap = 1.5 * tol;
            for frame in frames(k) {
                let at = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
                let plate = primitives::block(
                    &frame.at(at(-6.0, -6.0, -2.0)).unwrap(),
                    12.0 * k,
                    12.0 * k,
                    2.0 * k,
                )
                .unwrap();
                let top = face_on(&plate, at(0.0, 0.0, 0.0), frame.z(), k);
                for radius in [20.0, 5.0] {
                    for low in [(2.0, -1.5), (-3.0, 3.5)] {
                        let what = format!(
                            "radius {radius}, lowest at {low:?}, at {k}, frame at {:?}",
                            frame.origin()
                        );
                        // A spline paraboloid over the plate's top.
                        let bowl =
                            spline_sheet(&frame, 5.0 * k, gap, radius * k, (low.0 * k, low.1 * k));
                        let spline = bowl.faces[0].surface.clone();
                        let height = |x: Vec3| (x - frame.origin()).dot(frame.z());
                        let found = least_of((&bowl, 0), (&plate, top), tol);
                        let grid = sampled(&spline, height);
                        assert!(
                            (found - gap).abs() <= 1e-4 * tol && found <= grid + 1e-6 * tol,
                            "a spline bowl over a plane, {what}: found {found}, sampled {grid}, the least {gap}"
                        );
                        assert!(
                            grid >= gap - 1e-9 * k && grid - gap <= 1e-3 * tol,
                            "the sampling, {what}: {grid} against {gap}"
                        );
                        // A ball over a flat spline sheet, and over the plate.
                        let sheet = spline_sheet(&frame, 5.0 * k, 0.0, f64::INFINITY, (0.0, 0.0));
                        let flat = sheet.faces[0].surface.clone();
                        let centre = at(low.0, low.1, radius + 1.5 * 0.01);
                        let ball =
                            primitives::sphere(&frame.at(centre).unwrap(), radius * k).unwrap();
                        let off = |x: Vec3| x.distance(centre) - radius * k;
                        let found = least_of((&sheet, 0), (&ball, 0), tol);
                        let grid = sampled(&flat, off);
                        assert!(
                            (found - gap).abs() <= 1e-4 * tol && found <= grid + 1e-6 * tol,
                            "a ball over a spline sheet, {what}: found {found}, sampled {grid}, the least {gap}"
                        );
                        assert!(
                            grid >= gap - 1e-9 * k && grid - gap <= 1e-3 * tol,
                            "the sampling, {what}: {grid} against {gap}"
                        );
                        let found = least_of((&plate, top), (&ball, 0), tol);
                        assert!(
                            (found - gap).abs() <= 1e-9 * k,
                            "a ball over a plane, {what}: found {found}, the least {gap}"
                        );
                    }
                }
            }
        }
    }

    /// A look stopped at a cap with nothing found: the boolean refuses
    /// naming the cap, and interference is unknown (refused in the same
    /// words), never clear. Two balls 1.5 tolerances apart, which the look
    /// finds: with the real caps the union is zero thickness and the
    /// interference a touch; with a cap of no approach held, or of no work
    /// (so no pair looked at), nothing is found and both are refused for
    /// the cap. Two balls 3 tolerances apart: built as two parts and clear,
    /// as before. The reason names the caps as they are. At 1e-3, 1 and
    /// 1e3.
    #[test]
    fn a_look_cut_short_refuses_and_is_never_clear() {
        assert!(
            CUT_SHORT.contains(&format!("{MOST_WORK} units of work"))
                && CUT_SHORT.contains(&format!("{MOST_FOUND} approaches")),
            "the reason names the caps"
        );
        for k in SCALES {
            let tol = Tolerance::new(0.01 * k, 1e-9).unwrap();
            let ball = |x: f64| {
                primitives::sphere(&Frame::WORLD.at(Vec3::new(x, 0.0, 0.0)).unwrap(), 2.0 * k)
                    .unwrap()
            };
            let a = ball(0.0);
            let near = ball(4.0 * k + 1.5 * tol.linear);
            let far = ball(4.0 * k + 3.0 * tol.linear);
            CAPS.with(|c| c.set(None));
            let union = boolean(&a, &near, BooleanOp::Union, &tol);
            assert!(
                matches!(union, Err(KernelError::ZeroThickness { .. })),
                "at {k}: {:?}",
                union.as_ref().err()
            );
            assert!(
                matches!(
                    interference(&a, &near, &tol),
                    Ok(Interference::Touch { .. })
                ),
                "at {k}"
            );
            for caps in [(u64::MAX, 0), (0, usize::MAX)] {
                CAPS.with(|c| c.set(Some(caps)));
                let union = boolean(&a, &near, BooleanOp::Union, &tol);
                let between = interference(&a, &near, &tol);
                CAPS.with(|c| c.set(None));
                assert!(
                    matches!(&union, Err(KernelError::Unsupported { reason, .. }) if *reason == CUT_SHORT),
                    "at {k}, caps {caps:?}: {:?}",
                    union.as_ref().err()
                );
                assert!(
                    matches!(&between, Err(KernelError::Unsupported { reason, .. }) if *reason == CUT_SHORT),
                    "at {k}, caps {caps:?}: {between:?}"
                );
            }
            let union = boolean(&a, &far, BooleanOp::Union, &tol);
            assert!(
                union.as_ref().is_ok_and(|r| r.body.shells.len() == 2),
                "at {k}: {:?}",
                union.as_ref().err()
            );
            assert_eq!(
                interference(&a, &far, &tol).unwrap(),
                Interference::Clear,
                "at {k}"
            );
        }
    }

    /// A number in [0, 1) (SplitMix64).
    fn random(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// A point within `k` of `centre` along each axis, at random.
    fn random_point(state: &mut u64, centre: Vec3, k: f64) -> Vec3 {
        let x = 2.0 * random(state) - 1.0;
        let y = 2.0 * random(state) - 1.0;
        let z = 2.0 * random(state) - 1.0;
        centre + Vec3::new(x, y, z) * k
    }

    /// An edge of a kind (0 a line, 1 an arc of a circle, 2 an arc of an
    /// ellipse, 3 a piece of a cubic spline) of a size about `k`, within
    /// `k` of `centre`, at random; arcs of up to a whole turn.
    fn random_edge(kind: usize, state: &mut u64, centre: Vec3, k: f64) -> Edge {
        let origin = random_point(state, centre, k);
        let z = random_point(state, Vec3::ZERO, 1.0);
        let x = random_point(state, Vec3::ZERO, 1.0);
        let frame = Frame::new(origin, z, x).unwrap_or_else(|| Frame::WORLD.at(origin).unwrap());
        let start = PI * (2.0 * random(state) - 1.0);
        let sweep = 0.05 + (2.0 * PI - 0.05) * random(state);
        let (curve, range) = match kind {
            0 => {
                let along = random_point(state, Vec3::ZERO, 1.0);
                let along = if along.length() > 0.1 { along } else { Vec3::X };
                let length = k * (0.2 + 2.0 * random(state));
                (
                    Curve3::Line(Line::new(origin, along).unwrap()),
                    (0.0, length),
                )
            }
            1 => {
                let radius = k * (0.2 + 2.0 * random(state));
                (
                    Curve3::Circle(Circle::new(frame, radius).unwrap()),
                    (start, start + sweep),
                )
            }
            2 => {
                let a = k * (0.3 + 2.0 * random(state));
                let b = a * (0.1 + 0.9 * random(state));
                (
                    Curve3::Ellipse(Ellipse::new(frame, a, b).unwrap()),
                    (start, start + sweep),
                )
            }
            _ => {
                let points: Vec<Vec3> = (0..4).map(|_| random_point(state, origin, k)).collect();
                let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
                let lo = 0.3 * random(state);
                let hi = lo + (1.0 - lo) * (0.2 + 0.8 * random(state));
                (
                    Curve3::Nurbs(NurbsCurve::polynomial(3, knots, points).unwrap()),
                    (lo, hi),
                )
            }
        };
        Edge {
            curve,
            range,
            start: VertexId(0),
            end: VertexId(0),
        }
    }

    /// The least distance between two edges as dense sampling finds it:
    /// 401 points along each, each with its nearest point on the other (a
    /// bound from above).
    fn sampled_apart(e1: &Edge, e2: &Edge) -> f64 {
        let one_way = |a: &Edge, b: &Edge| {
            (0..=400)
                .map(|i| {
                    let t = a.range.0 + (a.range.1 - a.range.0) * f64::from(i) / 400.0;
                    let p = a.curve.eval(t);
                    outline::nearest(&b.curve, b.range, p).map_or(f64::INFINITY, |(_, d)| d)
                })
                .fold(f64::INFINITY, f64::min)
        };
        one_way(e1, e2).min(one_way(e2, e1))
    }

    /// The bound of two edges ([`edges_apart`]) never passes over a pair
    /// that comes nearer than its `near`: for lines, arcs of circles and of
    /// ellipses and pieces of cubic splines at random, every pair of kinds,
    /// crossed, near each other and apart, at 1e-3, 1 and 1e6, either way
    /// round, it says apart only where the least distance dense sampling
    /// finds (a bound from above of the least there is) is at least `near`
    /// (with `near` a thousandth above it: never). And it is of use: of the
    /// pairs with an arc of a circle and a line or another arc, it proves
    /// at least a quarter apart at half their least distance.
    #[test]
    fn the_bound_of_two_edges_never_passes_over_a_pair_that_near() {
        let mut state = 7_u64;
        let (mut tried, mut proven) = (0usize, 0usize);
        for k in [1e-3, 1.0, 1e6] {
            let centre = Vec3::new(3.0, -2.0, 5.0) * k;
            for one in 0..4 {
                for two in 0..4 {
                    for spread in [0.3, 1.0, 3.0] {
                        for _ in 0..4 {
                            let e1 = random_edge(one, &mut state, centre, k);
                            let e2 = random_edge(two, &mut state, centre, spread * k);
                            let least = sampled_apart(&e1, &e2);
                            assert!(least.is_finite(), "kinds {one} and {two}");
                            let work = Work::new();
                            for (i, fraction) in
                                [0.5, 0.9, 0.999, 1.001, 1.5].into_iter().enumerate()
                            {
                                let near = fraction * least;
                                if near <= 0.0 {
                                    continue;
                                }
                                let either = edges_apart(&e1, &e2, near, &work);
                                let apart = either || edges_apart(&e2, &e1, near, &work);
                                assert!(
                                    !apart || least >= near,
                                    "kinds {one} and {two} at {k}: apart at {near}, sampled {least}: {e1:?} {e2:?}"
                                );
                                if i == 0 && (one == 1 || two == 1) && one < 2 && two < 2 {
                                    tried += 1;
                                    proven += usize::from(either);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            4 * proven >= tried,
            "arcs proven apart at half their least distance: {proven} of {tried}"
        );
    }

    /// [`segment_gap`] is never more than the least distance between two
    /// segments, a segment and a point, or two segments nearly parallel
    /// (sampled densely, a bound from above), at 1e-3, 1 and 1e6.
    #[test]
    fn the_gap_of_two_segments_is_never_more_than_their_distance() {
        let mut state = 11_u64;
        for k in [1e-3, 1.0, 1e6] {
            let centre = Vec3::new(-4.0, 7.0, 2.0) * k;
            for case in 0..400 {
                let a0 = random_point(&mut state, centre, k);
                let mut a1 = random_point(&mut state, centre, k);
                let b0 = random_point(&mut state, centre, k);
                let mut b1 = random_point(&mut state, centre, k);
                match case % 4 {
                    1 => a1 = a0,
                    2 => b1 = b0 + (a1 - a0) * 0.7 + Vec3::new(1e-9, 0.0, 0.0) * k,
                    3 => b1 = b0 + (a1 - a0) * -1.3,
                    _ => {}
                }
                let gap = segment_gap(a0, a1, b0, b1);
                let mut least = f64::INFINITY;
                for i in 0..=200 {
                    for j in 0..=200 {
                        let p = a0.lerp(a1, f64::from(i) / 200.0);
                        let q = b0.lerp(b1, f64::from(j) / 200.0);
                        least = least.min(p.distance(q));
                    }
                }
                assert!(
                    gap >= 0.0 && gap <= least + 1e-12 * k,
                    "case {case} at {k}: the gap {gap}, sampled {least}"
                );
            }
        }
    }
}
