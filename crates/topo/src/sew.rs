//! Sewing: faces whose boundaries do not share their edges and vertices,
//! but meet within a gap, made into one body that shares them (the
//! decision of 2026-10-18, *Tolerant modelling*: sewing "merges topology
//! and measures gaps; it never moves geometry"; `docs/design/topology.md`,
//! Method › *Sewing*).
//!
//! **What is sewn.** Only *free* edges, those one loop uses once (a seam a
//! face uses twice, and an edge two faces share already, are left alone).
//! Two free edges are one when they run between the same places within
//! `distance` (their end vertices, either way round) and every one of
//! [`SAMPLES`] places inside each lies within `distance` of the other,
//! and when joining them keeps the boundary oriented: the two uses run
//! against each other, as the two uses of an edge of an oriented surface
//! do. Every pair that qualifies is a candidate; the candidates are taken
//! closest first (the larger of the gaps measured, then the edge ids), and
//! one is taken only while both its edges are still free and joining its
//! ends does not make one vertex of the two ends of any edge (sewing
//! never collapses an edge). So a narrow slot between two walls, whose
//! edges are each other's candidates too, is not sewn shut while each
//! wall's own partner is nearer.
//!
//! **What the result is.** The lower edge of a pair is kept with its curve
//! and range, the other goes into it; vertices joined through pairs are
//! one, the lowest of them with its point. Faces keep their ids; shells
//! joined by a pair become one (the lower one's faces first). Nothing is
//! moved. Then every edge kept from a pair is measured on the surfaces of
//! both its faces, proven over its whole length (`record_edge`), and every
//! vertex joined is measured against the ends of its edges
//! (`record_vertex`): a gap beyond the document's tolerance is recorded as
//! the entity's own tolerance; a gap beyond `cap` (the document's
//! `loosest`) refuses. Edges left free are reported with how far the
//! nearest free edge is ([`Sewn::open`]); in a solid they are the
//! caller's to refuse.
//!
//! **Bounded and deterministic.** Free edges are found through a grid of
//! their end vertices (cells `2 distance` wide, ordered maps), the
//! candidates tested are capped ([`MAX_CANDIDATES`]), and every order is
//! the order of ids or of numbers, never of a hash.

use crate::tolerant::{ToleranceError, record_edge, record_vertices};
use crate::{Body, EdgeId, FaceId, VertexId};
use craft3d_geom::Curve3;
use craft3d_math::{Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Places inside an edge, besides its ends, that must lie within the
/// sewing distance of the other edge of a pair: the ends say where an
/// edge runs, these that it runs the same way between them (two arcs
/// between the same points, an arc and its chord, are told apart).
pub const SAMPLES: u32 = 7;

/// The most pairs of free edges whose ends meet that are tested. A body
/// where more meet is refused ([`SewError::TooMany`]): every free edge of
/// a real body has a handful of candidates, so this is only reached by
/// thousands of edges ending at one place.
pub const MAX_CANDIDATES: usize = 1 << 22;

/// The most free-edge ends looked at, near the start of each free edge,
/// before the ends are compared: a body with more is refused
/// ([`SewError::TooMany`]); thousands of edges ending at one place (a fan
/// of facets) cost their square here and stay within it.
pub const MAX_PROBES: usize = 1 << 27;

/// The most open edges for which the nearest free edge is looked for
/// when reporting them (every pair is looked at, so this bounds the
/// report's work at its square).
const MAX_REPORTED_OPEN: usize = 4096;

/// A body sewn.
#[derive(Clone, Debug)]
pub struct Sewn {
    pub body: Body,
    /// Per edge of the body given, the edge of the result it is or went
    /// into, and whether it runs against that edge.
    pub edges: Vec<(EdgeId, bool)>,
    /// Per vertex of the body given, the vertex of the result it is or
    /// went into.
    pub vertices: Vec<VertexId>,
    /// Pairs of free edges joined.
    pub joined: usize,
    /// Vertices that went into another one.
    pub merged_vertices: usize,
    /// The largest distance measured between the two edges of a pair (at
    /// their ends and the places inside); 0 when nothing was joined.
    pub largest_gap: f64,
    /// The largest tolerance recorded on an entity sewing joined; 0 when
    /// every one lies within the document's tolerance.
    pub largest_tolerance: f64,
    /// Edges of the result still used once, each with the distance to the
    /// nearest other free edge (the larger of its two end gaps, the
    /// better way round), `None` when there is none or it was not looked
    /// for (more than `MAX_REPORTED_OPEN` open edges); in edge order.
    pub open: Vec<(EdgeId, Option<f64>)>,
}

impl Sewn {
    /// The open edge whose nearest partner is farthest (none counts as
    /// farthest), with that distance.
    pub fn worst_open(&self) -> Option<(EdgeId, Option<f64>)> {
        let mut worst: Option<(EdgeId, Option<f64>)> = None;
        for &(edge, gap) in &self.open {
            let farther = match (worst, gap) {
                (None, _) => true,
                (Some((_, Some(_))), None) => true,
                (Some((_, Some(w))), Some(g)) => g > w,
                (Some((_, None)), _) => false,
            };
            if farther {
                worst = Some((edge, gap));
            }
        }
        worst
    }
}

/// Why a body could not be sewn.
#[derive(Clone, Debug, PartialEq)]
pub enum SewError {
    /// The sewing distance is not positive and finite.
    Distance { value: f64 },
    /// More pairs of free edges meet than [`MAX_CANDIDATES`].
    TooMany { candidates: usize },
    /// A joined edge or vertex lies further off than the cap: `edge` or
    /// `vertex` names it (of the result), `measured` how far.
    Gap {
        edge: Option<EdgeId>,
        vertex: Option<VertexId>,
        measured: Option<f64>,
        cap: f64,
    },
    /// The body refers to an entity that does not exist (run `check`).
    Dangling,
}

impl fmt::Display for SewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SewError::Distance { value } => write!(
                f,
                "the sewing distance must be positive and finite, got {value:e}"
            ),
            SewError::TooMany { candidates } => write!(
                f,
                "more than {candidates} pairs of free edges meet at the same places; the body is not sewn"
            ),
            SewError::Gap {
                edge,
                vertex,
                measured,
                cap,
            } => {
                let what = match (edge, vertex) {
                    (Some(e), _) => format!("the sewn edge {}", e.0),
                    (None, Some(v)) => format!("the sewn vertex {}", v.0),
                    (None, None) => "an entity".to_string(),
                };
                match measured {
                    Some(m) => write!(
                        f,
                        "{what} lies {m:e} off what it joins, more than the cap {cap:e} on an entity's own tolerance"
                    ),
                    None => write!(
                        f,
                        "{what} could not be proven within the cap {cap:e} of what it joins"
                    ),
                }
            }
            SewError::Dangling => write!(f, "the body refers to an entity that does not exist"),
        }
    }
}

impl std::error::Error for SewError {}

/// One use of a free edge.
#[derive(Clone, Copy)]
struct Free {
    edge: EdgeId,
    /// The loop runs against the edge.
    reversed: bool,
}

/// A pair of free edges that may be one: `b` goes into `a`, against it
/// when `flip`.
#[derive(Clone, Copy)]
struct Candidate {
    gap: f64,
    a: EdgeId,
    b: EdgeId,
    flip: bool,
}

/// A cell of the grid of free edges' ends.
type Cell = (i64, i64, i64);

/// A free edge's end in a cell: (edge, that end, the other end, whether
/// that end is the edge's end).
type End = (u32, Vec3, Vec3, bool);

fn cell(p: Vec3, size: f64) -> Option<Cell> {
    let index = |x: f64| {
        let k = (x / size).floor();
        // Within what an i64 holds exactly and far from its ends.
        (k.is_finite() && k.abs() < 1e15).then_some(k as i64)
    };
    Some((index(p.x)?, index(p.y)?, index(p.z)?))
}

fn id32(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Union-find over vertices, the lowest id the root of each class.
struct Classes {
    parent: Vec<u32>,
    /// Per root, the edges (not closed) with an end in the class.
    ends: BTreeMap<u32, BTreeSet<u32>>,
}

impl Classes {
    fn root(&mut self, v: u32) -> u32 {
        let mut i = v;
        // Every step moves towards a root, so this ends.
        for _ in 0..=self.parent.len() {
            let Some(&up) = self.parent.get(i as usize) else {
                return i;
            };
            if up == i {
                return i;
            }
            let grand = self.parent.get(up as usize).copied().unwrap_or(up);
            if let Some(slot) = self.parent.get_mut(i as usize) {
                *slot = grand;
            }
            i = grand;
        }
        i
    }

    /// Whether joining the classes of each pair would make one vertex of
    /// both ends of an edge: the classes joined fall into groups (one, or
    /// two when the pairs share no class), and an edge with an end in two
    /// classes of one group would collapse.
    fn collapses(&mut self, pairs: [(u32, u32); 2]) -> bool {
        let roots = [
            [self.root(pairs[0].0), self.root(pairs[0].1)],
            [self.root(pairs[1].0), self.root(pairs[1].1)],
        ];
        let mut groups: Vec<BTreeSet<u32>> = roots
            .iter()
            .map(|r| r.iter().copied().collect::<BTreeSet<u32>>())
            .collect();
        if let [first, second] = groups.as_slice()
            && first.iter().any(|r| second.contains(r))
        {
            let all: BTreeSet<u32> = first.union(second).copied().collect();
            groups = vec![all];
        }
        for group in &groups {
            if group.len() < 2 {
                continue;
            }
            let mut seen: BTreeSet<u32> = BTreeSet::new();
            for r in group {
                for e in self.ends.get(r).into_iter().flatten() {
                    if !seen.insert(*e) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn join(&mut self, a: u32, b: u32) {
        let (x, y) = (self.root(a), self.root(b));
        if x == y {
            return;
        }
        let (low, high) = (x.min(y), x.max(y));
        if let Some(slot) = self.parent.get_mut(high as usize) {
            *slot = low;
        }
        let moved = self.ends.remove(&high).unwrap_or_default();
        self.ends.entry(low).or_default().extend(moved);
    }
}

/// The distance from `p` to the part of `curve` within `range`.
fn to_curve(curve: &Curve3, range: (f64, f64), p: Vec3) -> f64 {
    let (t0, t1) = range;
    let ends = curve.eval(t0).distance(p).min(curve.eval(t1).distance(p));
    let Ok(mut t) = curve.project(p) else {
        return ends;
    };
    if let Some(period) = curve.period()
        && period > 0.0
    {
        // Into the range, one period at a time from below.
        let turns = ((t0 - t) / period).ceil();
        if turns.is_finite() && turns.abs() < 1e6 {
            t += turns * period;
        }
    }
    if t >= t0 && t <= t1 {
        let d = curve.eval(t).distance(p);
        if d.is_finite() {
            return d.min(ends);
        }
    }
    ends
}

/// Points of the edge strictly inside it.
fn inside(curve: &Curve3, range: (f64, f64)) -> Vec<Vec3> {
    let (t0, t1) = range;
    (1..=SAMPLES)
        .map(|k| curve.eval(t0 + (t1 - t0) * f64::from(k) / f64::from(SAMPLES + 1)))
        .collect()
}

/// Sews `body`'s free edges that meet within `distance` (see the module
/// documentation), measuring and recording every gap above the
/// document's tolerance, refusing one above `cap`. A body without free
/// edges comes back as it is.
pub fn sew(body: &Body, tolerance: &Tolerance, distance: f64, cap: f64) -> Result<Sewn, SewError> {
    if !(distance.is_finite() && distance > 0.0) {
        return Err(SewError::Distance { value: distance });
    }
    let n_edges = body.edges.len();
    let n_vertices = body.vertices.len();
    // Uses of each edge.
    let mut uses: Vec<Vec<Free>> = vec![Vec::new(); n_edges];
    for face in &body.faces {
        for l in &face.loops {
            let l = body.loop_(*l).ok_or(SewError::Dangling)?;
            for c in &l.coedges {
                uses.get_mut(c.edge.0 as usize)
                    .ok_or(SewError::Dangling)?
                    .push(Free {
                        edge: c.edge,
                        reversed: c.reversed,
                    });
            }
        }
    }
    let free: Vec<Free> = uses
        .iter()
        .filter_map(|u| match u.as_slice() {
            [one] => Some(*one),
            _ => None,
        })
        .collect();
    let identity = || Sewn {
        body: body.clone(),
        edges: (0..n_edges).map(|i| (EdgeId(id32(i)), false)).collect(),
        vertices: (0..n_vertices).map(|i| VertexId(id32(i))).collect(),
        joined: 0,
        merged_vertices: 0,
        largest_gap: 0.0,
        largest_tolerance: 0.0,
        open: Vec::new(),
    };
    if free.is_empty() {
        return Ok(identity());
    }
    let point = |v: VertexId| body.vertex(v).map(|v| v.point).ok_or(SewError::Dangling);
    // Each free edge: its end points and places inside.
    struct Shape {
        ends: [Vec3; 2],
        inside: Vec<Vec3>,
    }
    let mut shapes: BTreeMap<u32, Shape> = BTreeMap::new();
    let size = 2.0 * distance;
    // Per cell, the free edges with an end in it: (edge, that end, the
    // other end, whether that end is the edge's end).
    let mut grid: BTreeMap<Cell, Vec<End>> = BTreeMap::new();
    for f in &free {
        let e = body.edge(f.edge).ok_or(SewError::Dangling)?;
        let ends = [point(e.start)?, point(e.end)?];
        let [first, last] = ends;
        for (p, q, at_end) in [(first, last, false), (last, first, true)] {
            if let Some(key) = cell(p, size) {
                grid.entry(key).or_default().push((f.edge.0, p, q, at_end));
            }
        }
        shapes.insert(
            f.edge.0,
            Shape {
                ends,
                inside: inside(&e.curve, e.range),
            },
        );
    }
    let free_of: BTreeMap<u32, Free> = free.iter().map(|f| (f.edge.0, *f)).collect();
    // Candidates: pairs whose ends meet, either way round.
    let mut candidates: Vec<Candidate> = Vec::new();
    let (mut probes, mut tested) = (0usize, 0usize);
    for (&a, sa) in &shapes {
        let [a_start, a_end] = sa.ends;
        let Some(key) = cell(a_start, size) else {
            continue;
        };
        // b's ends that meet a's (b's `meets` at a's start), in the order
        // of the cells and of the grid.
        let mut near: Vec<(u32, bool, f64)> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let k = (key.0 + dx, key.1 + dy, key.2 + dz);
                    for &(b, meets, other, at_end) in grid.get(&k).map_or(&[][..], Vec::as_slice) {
                        probes += 1;
                        if probes > MAX_PROBES {
                            return Err(SewError::TooMany {
                                candidates: MAX_PROBES,
                            });
                        }
                        if b <= a {
                            continue;
                        }
                        let gap = a_start.distance(meets).max(a_end.distance(other));
                        if gap <= distance {
                            near.push((b, at_end, gap));
                        }
                    }
                }
            }
        }
        for (b, flip, end_gap) in near {
            tested += 1;
            if tested > MAX_CANDIDATES {
                return Err(SewError::TooMany {
                    candidates: MAX_CANDIDATES,
                });
            }
            let (Some(sb), Some(fa), Some(fb)) = (shapes.get(&b), free_of.get(&a), free_of.get(&b))
            else {
                continue;
            };
            // Oriented: the uses run against each other.
            let (Some(ea), Some(eb)) = (body.edge(fa.edge), body.edge(fb.edge)) else {
                return Err(SewError::Dangling);
            };
            let flip = if ea.start == ea.end || a_start.distance(a_end) <= distance {
                // Both ends at one place (a closed edge, or one within the
                // distance of closing): which way b runs is the way its
                // curve runs near a's middle.
                let mid = 0.5 * (ea.range.0 + ea.range.1);
                let (p, ta) = ea.curve.derivatives(mid);
                let Ok(t) = eb.curve.project(p) else {
                    continue;
                };
                let tb = eb.curve.derivatives(t).1;
                ta.dot(tb) < 0.0
            } else {
                flip
            };
            if (fb.reversed ^ flip) == fa.reversed {
                continue;
            }
            let mut gap = end_gap;
            for p in &sa.inside {
                gap = gap.max(to_curve(&eb.curve, eb.range, *p));
                if gap.is_nan() || gap > distance {
                    break;
                }
            }
            if gap <= distance {
                for p in &sb.inside {
                    gap = gap.max(to_curve(&ea.curve, ea.range, *p));
                    if gap.is_nan() || gap > distance {
                        break;
                    }
                }
            }
            if gap <= distance {
                candidates.push(Candidate {
                    gap,
                    a: EdgeId(a),
                    b: EdgeId(b),
                    flip,
                });
            }
        }
    }
    // Closest first; equal gaps in the order of the edges.
    candidates.sort_by(|x, y| {
        x.gap
            .total_cmp(&y.gap)
            .then(x.a.cmp(&y.a))
            .then(x.b.cmp(&y.b))
            .then(x.flip.cmp(&y.flip))
    });
    let mut classes = Classes {
        parent: (0..id32(n_vertices)).collect(),
        ends: BTreeMap::new(),
    };
    for (i, e) in body.edges.iter().enumerate() {
        if e.start != e.end {
            for v in [e.start, e.end] {
                classes.ends.entry(v.0).or_default().insert(id32(i));
            }
        }
    }
    let mut taken: BTreeSet<u32> = BTreeSet::new();
    // Edge b → (edge a it goes into, against it).
    let mut into: BTreeMap<u32, (EdgeId, bool)> = BTreeMap::new();
    let mut largest_gap = 0.0_f64;
    for c in &candidates {
        if taken.contains(&c.a.0) || taken.contains(&c.b.0) {
            continue;
        }
        let (Some(ea), Some(eb)) = (body.edge(c.a), body.edge(c.b)) else {
            return Err(SewError::Dangling);
        };
        let (b_start, b_end) = if c.flip {
            (eb.end, eb.start)
        } else {
            (eb.start, eb.end)
        };
        let pairs = [(ea.start.0, b_start.0), (ea.end.0, b_end.0)];
        if classes.collapses(pairs) {
            continue;
        }
        for (x, y) in pairs {
            classes.join(x, y);
        }
        taken.insert(c.a.0);
        taken.insert(c.b.0);
        into.insert(c.b.0, (c.a, c.flip));
        largest_gap = largest_gap.max(c.gap);
    }
    if into.is_empty() {
        let mut out = identity();
        out.open = open_edges(&out.body)?;
        return Ok(out);
    }
    build(body, tolerance, cap, &mut classes, &into, largest_gap)
}

/// The sewn body from the pairs taken.
fn build(
    body: &Body,
    tolerance: &Tolerance,
    cap: f64,
    classes: &mut Classes,
    into: &BTreeMap<u32, (EdgeId, bool)>,
    largest_gap: f64,
) -> Result<Sewn, SewError> {
    let n_vertices = body.vertices.len();
    // Vertices: one per class, in the order of the lowest of each.
    let mut vertex_map: Vec<VertexId> = Vec::with_capacity(n_vertices);
    let mut new_of_root: BTreeMap<u32, VertexId> = BTreeMap::new();
    let mut out = Body::new(body.kind);
    let mut merged_vertices = 0;
    for (i, v) in body.vertices.iter().enumerate() {
        let root = classes.root(id32(i));
        match new_of_root.get(&root) {
            Some(&nv) => {
                vertex_map.push(nv);
                merged_vertices += 1;
            }
            None => {
                let nv = out.add_vertex(v.point);
                new_of_root.insert(root, nv);
                vertex_map.push(nv);
            }
        }
    }
    let vmap = |v: VertexId| {
        vertex_map
            .get(v.0 as usize)
            .copied()
            .ok_or(SewError::Dangling)
    };
    // Edges: those that go into no other, in order.
    let mut edge_map: Vec<(EdgeId, bool)> = vec![(EdgeId(u32::MAX), false); body.edges.len()];
    for (i, e) in body.edges.iter().enumerate() {
        if into.contains_key(&id32(i)) {
            continue;
        }
        let ne = out.add_edge(e.curve.clone(), e.range, vmap(e.start)?, vmap(e.end)?);
        if let Some(slot) = edge_map.get_mut(i) {
            *slot = (ne, false);
        }
    }
    for (&b, &(a, flip)) in into {
        let kept = edge_map
            .get(a.0 as usize)
            .map(|m| m.0)
            .ok_or(SewError::Dangling)?;
        if let Some(slot) = edge_map.get_mut(b as usize) {
            *slot = (kept, flip);
        }
    }
    for l in &body.loops {
        let mut coedges = Vec::with_capacity(l.coedges.len());
        for c in &l.coedges {
            let &(edge, flip) = edge_map.get(c.edge.0 as usize).ok_or(SewError::Dangling)?;
            coedges.push(crate::Coedge {
                edge,
                reversed: c.reversed ^ flip,
            });
        }
        out.add_loop(coedges);
    }
    for f in &body.faces {
        out.add_face(f.surface.clone(), f.reversed, f.loops.clone());
    }
    // Shells joined by a pair become one, the lower one's faces first.
    let mut shell_of: BTreeMap<u32, usize> = BTreeMap::new();
    for (s, shell) in body.shells.iter().enumerate() {
        for f in &shell.faces {
            shell_of.entry(f.0).or_insert(s);
        }
    }
    let mut shell_parent: Vec<usize> = (0..body.shells.len()).collect();
    fn shell_root(parent: &mut [usize], mut i: usize) -> usize {
        for _ in 0..=parent.len() {
            let up = parent.get(i).copied().unwrap_or(i);
            if up == i {
                break;
            }
            i = up;
        }
        i
    }
    let faces_of = crate::edge_faces(body);
    for (&b, &(a, _)) in into {
        let fa = faces_of.get(a.0 as usize).and_then(|f| f.first());
        let fb = faces_of.get(b as usize).and_then(|f| f.first());
        if let (Some(fa), Some(fb)) = (fa, fb)
            && let (Some(&sa), Some(&sb)) = (shell_of.get(&fa.0), shell_of.get(&fb.0))
        {
            let (x, y) = (
                shell_root(&mut shell_parent, sa),
                shell_root(&mut shell_parent, sb),
            );
            if x != y {
                let (low, high) = (x.min(y), x.max(y));
                if let Some(slot) = shell_parent.get_mut(high) {
                    *slot = low;
                }
            }
        }
    }
    let mut shells: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for (s, shell) in body.shells.iter().enumerate() {
        let r = shell_root(&mut shell_parent, s);
        shells
            .entry(r)
            .or_default()
            .extend(shell.faces.iter().copied());
    }
    for faces in shells.into_values() {
        out.add_shell(faces);
    }
    // The table of the body given, through the maps (a joined entity the
    // loosest of what went into it).
    for (v, t) in body.tolerances.vertices() {
        let nv = vmap(v)?;
        if out.vertex_tolerance(nv).is_none_or(|had| had < t) {
            out.set_vertex_tolerance(nv, t)
                .map_err(|_| SewError::Dangling)?;
        }
    }
    for (e, t) in body.tolerances.edges() {
        let &(ne, _) = edge_map.get(e.0 as usize).ok_or(SewError::Dangling)?;
        if out.edge_tolerance(ne).is_none_or(|had| had < t) {
            out.set_edge_tolerance(ne, t)
                .map_err(|_| SewError::Dangling)?;
        }
    }
    // Measure what was joined: the kept edges of pairs, then every vertex
    // of a class of more than one and the ends of the kept edges.
    let mut largest_tolerance = 0.0_f64;
    let joined_edges: BTreeSet<EdgeId> = into
        .values()
        .filter_map(|(a, _)| edge_map.get(a.0 as usize).map(|m| m.0))
        .collect();
    let gap_error = |e: ToleranceError, edge: Option<EdgeId>, vertex: Option<VertexId>| match e {
        ToleranceError::AboveCap { measured, cap } => SewError::Gap {
            edge,
            vertex,
            measured: Some(measured / crate::RECORD_MARGIN),
            cap,
        },
        ToleranceError::EdgeNotMeasured { cap, .. } => SewError::Gap {
            edge,
            vertex,
            measured: None,
            cap,
        },
        _ => SewError::Dangling,
    };
    for &e in &joined_edges {
        if let Some(t) =
            record_edge(&mut out, e, tolerance, cap).map_err(|x| gap_error(x, Some(e), None))?
        {
            largest_tolerance = largest_tolerance.max(t);
        }
    }
    let mut members: BTreeMap<VertexId, usize> = BTreeMap::new();
    for nv in &vertex_map {
        *members.entry(*nv).or_default() += 1;
    }
    let mut joined_vertices: BTreeSet<VertexId> = members
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(v, _)| v)
        .collect();
    for &e in &joined_edges {
        if let Some(edge) = out.edge(e) {
            joined_vertices.insert(edge.start);
            joined_vertices.insert(edge.end);
        }
    }
    let joined_vertices: Vec<VertexId> = joined_vertices.into_iter().collect();
    for (_, t) in record_vertices(&mut out, &joined_vertices, tolerance, cap)
        .map_err(|(v, x)| gap_error(x, None, Some(v)))?
    {
        if let Some(t) = t {
            largest_tolerance = largest_tolerance.max(t);
        }
    }
    let open = open_edges(&out)?;
    Ok(Sewn {
        body: out,
        edges: edge_map,
        vertices: vertex_map,
        joined: into.len(),
        merged_vertices,
        largest_gap,
        largest_tolerance,
        open,
    })
}

/// The edges of `sewn` used once, each with the distance to the nearest
/// other one.
fn open_edges(sewn: &Body) -> Result<Vec<(EdgeId, Option<f64>)>, SewError> {
    let uses = crate::edge_uses(sewn);
    let open: Vec<EdgeId> = uses
        .iter()
        .enumerate()
        .filter(|(_, u)| u.len() == 1)
        .map(|(i, _)| EdgeId(id32(i)))
        .collect();
    let ends = |e: EdgeId| -> Result<[Vec3; 2], SewError> {
        let edge = sewn.edge(e).ok_or(SewError::Dangling)?;
        let p = |v: VertexId| sewn.vertex(v).map(|v| v.point).ok_or(SewError::Dangling);
        Ok([p(edge.start)?, p(edge.end)?])
    };
    let mut points = Vec::with_capacity(open.len());
    for &e in &open {
        points.push(ends(e)?);
    }
    let look = open.len() <= MAX_REPORTED_OPEN;
    let mut out = Vec::with_capacity(open.len());
    for (i, &e) in open.iter().enumerate() {
        let nearest = if look {
            let Some(a) = points.get(i) else {
                return Err(SewError::Dangling);
            };
            points
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, b)| {
                    let same = a[0].distance(b[0]).max(a[1].distance(b[1]));
                    let against = a[0].distance(b[1]).max(a[1].distance(b[0]));
                    same.min(against)
                })
                .filter(|d| d.is_finite())
                .reduce(f64::min)
        } else {
            None
        };
        out.push((e, nearest));
    }
    Ok(out)
}
