//! The facets of a crease pattern: the planar arrangement of the sheet's
//! boundary and its creases (Demaine and O'Rourke 2007, §11.1: a crease
//! pattern is a planar straight-line graph whose faces are the facets).
//!
//! Every segment is split where another one crosses it or ends on it,
//! points within the tolerance are one vertex, and the faces are traced
//! on the half-edges: at the end of a half-edge the next one is the edge
//! that comes before its twin counter-clockwise, so every face is walked
//! with its inside on the left (a bounded face counter-clockwise, the
//! boundary of a hole or of the outside clockwise). A clockwise cycle is
//! the hole of the smallest counter-clockwise cycle around it, or the
//! outside. A face lies in the sheet when its boundary edges run the way
//! the sheet's loops run (the sheet is on their left), or when it has
//! none (it is bounded by creases, which lie inside the sheet: checked
//! first, at the middle of every piece).

use super::{
    BoundarySegment, Crease, CreaseKind, FoldError, MAX_BOUNDARY, MAX_CREASES, P2, cross, dist,
    inside_loops, sub, to_segment,
};
use std::collections::BTreeMap;

/// What an edge of the arrangement is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// The sheet's boundary.
    Boundary,
    /// Crease number `.0` of the pattern (a fold or a cut).
    Crease(usize),
}

/// A straight edge of the arrangement between two vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// For a boundary edge, in the direction its loop runs (the sheet on
    /// the left); for a crease, as the crease was drawn.
    pub from: usize,
    pub to: usize,
    /// The curve it is a piece of.
    pub source: u64,
    /// Which piece of that curve, counted from its start in its own
    /// direction.
    pub piece: u32,
    pub kind: EdgeKind,
}

/// A face of the arrangement inside the sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    /// Half-edges of its outer boundary, counter-clockwise.
    pub outer: Vec<usize>,
    /// Half-edges of each hole, clockwise.
    pub holes: Vec<Vec<usize>>,
    /// Its area (the holes taken away).
    pub area: f64,
}

/// A fold crease between two facets: the edge, the facet on its left (as
/// the edge runs) and the one on its right, and the crease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hinge {
    pub edge: usize,
    pub left: usize,
    pub right: usize,
    pub crease: usize,
}

/// The arrangement of a crease pattern. Half-edge `h` runs along edge
/// `h / 2`, from its `from` to its `to` when `h` is even and back when it
/// is odd.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    pub vertices: Vec<P2>,
    pub edges: Vec<Edge>,
    pub facets: Vec<Facet>,
    /// One per fold crease piece (not a cut), in the order of the edges.
    pub hinges: Vec<Hinge>,
    pub creases: Vec<Crease>,
    /// Per half-edge: the facet on its left; `None` outside the sheet.
    pub face_of: Vec<Option<usize>>,
    /// Per half-edge: the next half-edge round the face on its left.
    pub next: Vec<usize>,
    /// Per vertex: the half-edges that leave it, counter-clockwise from
    /// the direction −x.
    pub around: Vec<Vec<usize>>,
    /// Per half-edge: where it stands in `around` of the vertex it leaves.
    pub position: Vec<usize>,
    pub tolerance: f64,
}

impl Arrangement {
    /// The vertex half-edge `h` leaves.
    pub fn tail(&self, h: usize) -> Option<usize> {
        let e = self.edges.get(h / 2)?;
        Some(if h.is_multiple_of(2) { e.from } else { e.to })
    }

    /// The vertex half-edge `h` arrives at.
    pub fn head(&self, h: usize) -> Option<usize> {
        self.tail(h ^ 1)
    }

    pub fn point(&self, v: usize) -> Option<P2> {
        self.vertices.get(v).copied()
    }

    /// The corners of a cycle of half-edges, in order.
    pub fn polygon(&self, cycle: &[usize]) -> Vec<P2> {
        cycle
            .iter()
            .filter_map(|&h| self.tail(h).and_then(|v| self.point(v)))
            .collect()
    }

    /// The kind of the crease an edge belongs to; `None` for the
    /// boundary.
    pub fn crease_kind(&self, edge: usize) -> Option<CreaseKind> {
        match self.edges.get(edge)?.kind {
            EdgeKind::Boundary => None,
            EdgeKind::Crease(c) => Some(self.creases.get(c)?.kind),
        }
    }

    /// True for an edge the sheet is not joined across: the boundary or a
    /// cut.
    pub fn separates(&self, edge: usize) -> bool {
        !matches!(
            self.crease_kind(edge),
            Some(CreaseKind::Mountain | CreaseKind::Valley)
        )
    }

    /// The facet that holds `p` strictly inside (further than the
    /// tolerance from its edges).
    pub fn facet_at(&self, p: P2) -> Option<usize> {
        self.facets.iter().position(|facet| {
            let rings: Vec<Vec<P2>> = std::iter::once(&facet.outer)
                .chain(&facet.holes)
                .map(|c| self.polygon(c))
                .collect();
            let clear = rings.iter().all(|ring| {
                let n = ring.len();
                (0..n).all(|i| match (ring.get(i), ring.get((i + 1) % n)) {
                    (Some(&a), Some(&b)) => to_segment(p, a, b).0 > self.tolerance,
                    _ => true,
                })
            });
            clear && inside_loops(p, rings.iter().map(Vec::as_slice))
        })
    }
}

/// A segment to split: a piece of the boundary or a crease.
struct Seg {
    a: P2,
    b: P2,
    id: u64,
    /// The curve runs from `b` to `a`.
    own_reversed: bool,
    kind: EdgeKind,
}

impl Seg {
    fn len(&self) -> f64 {
        dist(self.a, self.b)
    }
}

/// Divides the sheet bounded by `boundary` (closed loops, each segment in
/// its loop's direction) along `creases` into facets. `tol` is the
/// document's linear tolerance.
pub fn arrange(
    boundary: &[BoundarySegment],
    creases: &[Crease],
    tol: f64,
) -> Result<Arrangement, FoldError> {
    if !(tol.is_finite() && tol > 0.0) {
        return Err(FoldError::Tolerance);
    }
    if creases.len() > MAX_CREASES {
        return Err(FoldError::TooMany {
            what: "creases",
            limit: MAX_CREASES,
        });
    }
    if boundary.len() > MAX_BOUNDARY {
        return Err(FoldError::TooMany {
            what: "straight pieces of the sheet's boundary",
            limit: MAX_BOUNDARY,
        });
    }
    let mut segs = Vec::with_capacity(boundary.len() + creases.len());
    for b in boundary {
        segs.push(Seg {
            a: b.start,
            b: b.end,
            id: b.id,
            own_reversed: b.reversed,
            kind: EdgeKind::Boundary,
        });
    }
    for (i, c) in creases.iter().enumerate() {
        segs.push(Seg {
            a: c.start,
            b: c.end,
            id: c.id,
            own_reversed: false,
            kind: EdgeKind::Crease(i),
        });
    }
    for s in &segs {
        if !(s.a.iter().chain(&s.b).all(|x| x.is_finite())) {
            return Err(FoldError::NotFinite { id: s.id });
        }
        if s.len() <= tol {
            return Err(match s.kind {
                EdgeKind::Crease(_) => FoldError::TooShort { id: s.id },
                EdgeKind::Boundary => FoldError::Broken {
                    reason: "a piece of the boundary is no longer than the tolerance",
                },
            });
        }
    }

    // Where each segment is split: parameter and point.
    let mut splits: Vec<Vec<(f64, P2)>> =
        segs.iter().map(|s| vec![(0.0, s.a), (1.0, s.b)]).collect();
    let min_x = |s: &Seg| s.a[0].min(s.b[0]);
    let max_x = |s: &Seg| s.a[0].max(s.b[0]);
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&i, &j| {
        let (Some(a), Some(b)) = (segs.get(i), segs.get(j)) else {
            return i.cmp(&j);
        };
        min_x(a).total_cmp(&min_x(b)).then(i.cmp(&j))
    });
    for (k, &i) in order.iter().enumerate() {
        let Some(si) = segs.get(i) else { continue };
        for &j in order.iter().skip(k + 1) {
            let Some(sj) = segs.get(j) else { continue };
            if min_x(sj) > max_x(si) + tol {
                break;
            }
            let (lo_i, hi_i) = (si.a[1].min(si.b[1]), si.a[1].max(si.b[1]));
            let (lo_j, hi_j) = (sj.a[1].min(sj.b[1]), sj.a[1].max(sj.b[1]));
            if lo_j > hi_i + tol || lo_i > hi_j + tol {
                continue;
            }
            let (on_i, on_j) = meet(si, sj, tol)?;
            if let Some(list) = splits.get_mut(i) {
                list.extend(on_i);
            }
            if let Some(list) = splits.get_mut(j) {
                list.extend(on_j);
            }
        }
    }

    // One vertex per cluster of points within the tolerance.
    let mut points: Vec<(u64, P2)> = Vec::new();
    for (seg, list) in segs.iter().zip(&splits) {
        for &(_, p) in list {
            points.push((seg.id, p));
        }
    }
    let cluster = clusters(&points, tol)?;
    let mut vertex_of_rep: BTreeMap<usize, usize> = BTreeMap::new();
    let mut vertices = Vec::new();
    for (i, &rep) in cluster.iter().enumerate() {
        if rep == i
            && let Some(&(_, p)) = points.get(i)
        {
            vertex_of_rep.insert(rep, vertices.len());
            vertices.push(p);
        }
    }
    let vertex_of_point =
        |i: usize| -> Option<usize> { vertex_of_rep.get(cluster.get(i)?).copied() };

    // The edges: each segment between the vertices on it.
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    let mut next_point = 0usize;
    for (s, list) in splits.iter().enumerate() {
        let Some(seg) = segs.get(s) else { continue };
        let mut on: Vec<(f64, usize)> = Vec::with_capacity(list.len());
        for (k, &(t, _)) in list.iter().enumerate() {
            let v = vertex_of_point(next_point + k).ok_or(FoldError::Broken {
                reason: "a point lost its vertex",
            })?;
            on.push((t, v));
        }
        next_point += list.len();
        on.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        on.dedup_by(|b, a| a.1 == b.1);
        let pieces: Vec<(usize, usize)> = on
            .windows(2)
            .filter_map(|w| match w {
                [a, b] if a.1 != b.1 => Some((a.1, b.1)),
                _ => None,
            })
            .collect();
        let count = pieces.len();
        for (j, (u, v)) in pieces.into_iter().enumerate() {
            let key = (u.min(v), u.max(v));
            if let Some(&other) = seen.get(&key) {
                return Err(FoldError::Overlap {
                    a: other,
                    b: seg.id,
                });
            }
            seen.insert(key, seg.id);
            let piece = if seg.own_reversed { count - 1 - j } else { j };
            edges.push(Edge {
                from: u,
                to: v,
                source: seg.id,
                piece: u32::try_from(piece).unwrap_or(u32::MAX),
                kind: seg.kind,
            });
        }
    }

    // Every piece of a crease lies inside the sheet.
    let rings = boundary_rings(boundary);
    for e in &edges {
        if let EdgeKind::Crease(_) = e.kind {
            let (Some(a), Some(b)) = (vertices.get(e.from), vertices.get(e.to)) else {
                continue;
            };
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            if !inside_loops(mid, rings.iter().map(Vec::as_slice)) {
                return Err(FoldError::Outside { id: e.source });
            }
        }
    }

    // Half-edges round each vertex, counter-clockwise.
    let halves = edges.len() * 2;
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); vertices.len()];
    let angle_of = |h: usize| -> f64 {
        let Some(e) = edges.get(h / 2) else {
            return 0.0;
        };
        let (from, to) = if h.is_multiple_of(2) {
            (e.from, e.to)
        } else {
            (e.to, e.from)
        };
        match (vertices.get(from), vertices.get(to)) {
            (Some(a), Some(b)) => {
                let d = sub(*b, *a);
                d[1].atan2(d[0])
            }
            _ => 0.0,
        }
    };
    for h in 0..halves {
        let Some(e) = edges.get(h / 2) else { continue };
        let tail = if h.is_multiple_of(2) { e.from } else { e.to };
        if let Some(list) = around.get_mut(tail) {
            list.push(h);
        }
    }
    let mut position = vec![0usize; halves];
    for list in &mut around {
        list.sort_by(|&a, &b| angle_of(a).total_cmp(&angle_of(b)).then(a.cmp(&b)));
        for (i, &h) in list.iter().enumerate() {
            if let Some(slot) = position.get_mut(h) {
                *slot = i;
            }
        }
    }
    let mut next = vec![0usize; halves];
    for h in 0..halves {
        let twin = h ^ 1;
        let Some(e) = edges.get(h / 2) else { continue };
        let head = if h.is_multiple_of(2) { e.to } else { e.from };
        let Some(list) = around.get(head) else {
            continue;
        };
        let deg = list.len();
        let i = position.get(twin).copied().unwrap_or(0);
        if let (Some(&n), Some(slot)) = (list.get((i + deg - 1) % deg.max(1)), next.get_mut(h)) {
            *slot = n;
        }
    }

    // The cycles of half-edges, with their signed areas.
    let mut cycle_of = vec![usize::MAX; halves];
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    for start in 0..halves {
        if cycle_of.get(start).copied() != Some(usize::MAX) {
            continue;
        }
        let index = cycles.len();
        let mut cycle = Vec::new();
        let mut h = start;
        loop {
            if cycle.len() > halves {
                return Err(FoldError::Broken {
                    reason: "a face of the arrangement does not close",
                });
            }
            if let Some(slot) = cycle_of.get_mut(h) {
                *slot = index;
            }
            cycle.push(h);
            h = next.get(h).copied().unwrap_or(start);
            if h == start {
                break;
            }
        }
        cycles.push(cycle);
    }
    let polygon = |cycle: &[usize]| -> Vec<P2> {
        cycle
            .iter()
            .filter_map(|&h| {
                let e = edges.get(h / 2)?;
                vertices
                    .get(if h.is_multiple_of(2) { e.from } else { e.to })
                    .copied()
            })
            .collect()
    };
    let measures: Vec<(f64, f64)> = cycles
        .iter()
        .map(|c| {
            let ring = polygon(c);
            (signed_area(&ring), perimeter(&ring))
        })
        .collect();

    // The connected pieces of the graph. Each has one clockwise cycle,
    // round its outside; it is the hole of the smallest counter-clockwise
    // cycle of another piece around it, or the outside of everything. (A
    // cycle of the same piece shares its vertices, so containment is only
    // asked of the others, whose edges its vertices are not on.)
    let mut piece_of = Union::new(vertices.len());
    for e in &edges {
        piece_of.join(e.from, e.to);
    }
    let piece_of_cycle: Vec<usize> = cycles
        .iter()
        .map(|c| {
            let v = c
                .first()
                .and_then(|&h| {
                    edges
                        .get(h / 2)
                        .map(|e| if h.is_multiple_of(2) { e.from } else { e.to })
                })
                .unwrap_or(0);
            piece_of.find(v)
        })
        .collect();
    let mut holes_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, (area, length)) in measures.iter().enumerate() {
        if area.abs() <= tol * length {
            let crease =
                cycles
                    .get(i)
                    .into_iter()
                    .flatten()
                    .find_map(|&h| match edges.get(h / 2)?.kind {
                        EdgeKind::Crease(_) => edges.get(h / 2).map(|e| e.source),
                        EdgeKind::Boundary => None,
                    });
            return Err(match crease {
                Some(id) => FoldError::Dangling { id },
                None => FoldError::Broken {
                    reason: "the boundary encloses no area",
                },
            });
        }
        if *area > 0.0 {
            continue;
        }
        let Some(&first) = cycles.get(i).and_then(|c| c.first()) else {
            continue;
        };
        let Some(p) = edges
            .get(first / 2)
            .and_then(|e| {
                vertices.get(if first.is_multiple_of(2) {
                    e.from
                } else {
                    e.to
                })
            })
            .copied()
        else {
            continue;
        };
        let mut best: Option<(usize, f64)> = None;
        for (j, (outer, _)) in measures.iter().enumerate() {
            if *outer <= 0.0
                || best.is_some_and(|(_, a)| a <= *outer)
                || piece_of_cycle.get(j) == piece_of_cycle.get(i)
            {
                continue;
            }
            let ring = cycles.get(j).map(|c| polygon(c)).unwrap_or_default();
            if inside_loops(p, [ring.as_slice()]) {
                best = Some((j, *outer));
            }
        }
        if let Some((j, _)) = best {
            holes_of.entry(j).or_default().push(i);
        }
    }

    // The faces inside the sheet.
    let mut facets = Vec::new();
    let mut face_of: Vec<Option<usize>> = vec![None; halves];
    for (i, (area, length)) in measures.iter().enumerate() {
        if *area <= 0.0 {
            continue;
        }
        let holes: Vec<usize> = holes_of.get(&i).cloned().unwrap_or_default();
        let all: Vec<usize> = std::iter::once(i).chain(holes.iter().copied()).collect();
        let (mut ins, mut outs) = (0usize, 0usize);
        for &c in &all {
            for &h in cycles.get(c).into_iter().flatten() {
                if let Some(Edge {
                    kind: EdgeKind::Boundary,
                    ..
                }) = edges.get(h / 2)
                {
                    if h.is_multiple_of(2) {
                        ins += 1;
                    } else {
                        outs += 1;
                    }
                }
            }
        }
        if ins > 0 && outs > 0 {
            return Err(FoldError::Broken {
                reason: "a face lies on both sides of the boundary",
            });
        }
        if outs > 0 {
            continue;
        }
        let index = facets.len();
        if *area <= tol * length {
            return Err(FoldError::Sliver { facet: index });
        }
        let hole_area: f64 = holes
            .iter()
            .filter_map(|&c| measures.get(c))
            .map(|(a, _)| a.abs())
            .sum();
        for &c in &all {
            for &h in cycles.get(c).into_iter().flatten() {
                if let Some(slot) = face_of.get_mut(h) {
                    *slot = Some(index);
                }
            }
        }
        facets.push(Facet {
            outer: cycles.get(i).cloned().unwrap_or_default(),
            holes: holes
                .iter()
                .filter_map(|&c| cycles.get(c).cloned())
                .collect(),
            area: area - hole_area,
        });
    }

    // The hinges, and the creases that divide nothing.
    let mut hinges = Vec::new();
    for (e, edge) in edges.iter().enumerate() {
        let EdgeKind::Crease(c) = edge.kind else {
            continue;
        };
        let (Some(&left), Some(&right)) = (face_of.get(2 * e), face_of.get(2 * e + 1)) else {
            continue;
        };
        let (Some(left), Some(right)) = (left, right) else {
            return Err(FoldError::Outside { id: edge.source });
        };
        let kind = creases.get(c).map(|c| c.kind);
        if left == right {
            return Err(match kind {
                Some(CreaseKind::Cut) => FoldError::Slit { id: edge.source },
                _ => FoldError::Dangling { id: edge.source },
            });
        }
        if kind != Some(CreaseKind::Cut) {
            hinges.push(Hinge {
                edge: e,
                left,
                right,
                crease: c,
            });
        }
    }
    if facets.is_empty() {
        return Err(FoldError::Broken {
            reason: "the sheet has no facets",
        });
    }
    Ok(Arrangement {
        vertices,
        edges,
        facets,
        hinges,
        creases: creases.to_vec(),
        face_of,
        next,
        around,
        position,
        tolerance: tol,
    })
}

/// Where segments `i` and `j` cut each other: the split points on each.
/// Refuses two that lie along each other.
#[allow(clippy::type_complexity)]
fn meet(i: &Seg, j: &Seg, tol: f64) -> Result<(Vec<(f64, P2)>, Vec<(f64, P2)>), FoldError> {
    let (li, lj) = (i.len(), j.len());
    let interior = |t: f64, len: f64| t * len > tol && (1.0 - t) * len > tol;
    let mut on_i = Vec::new();
    let mut on_j = Vec::new();
    let mut touching: Vec<P2> = Vec::new();
    for p in [j.a, j.b] {
        let (d, t) = to_segment(p, i.a, i.b);
        if d <= tol {
            touching.push(p);
            if interior(t, li) {
                on_i.push((t, p));
            }
        }
    }
    for p in [i.a, i.b] {
        let (d, t) = to_segment(p, j.a, j.b);
        if d <= tol {
            touching.push(p);
            if interior(t, lj) {
                on_j.push((t, p));
            }
        }
    }
    for (k, p) in touching.iter().enumerate() {
        for q in touching.iter().skip(k + 1) {
            if dist(*p, *q) > tol {
                return Err(FoldError::Overlap { a: i.id, b: j.id });
            }
        }
    }
    let di = sub(i.b, i.a);
    let dj = sub(j.b, j.a);
    let c = cross(di, dj);
    if c.abs() > 16.0 * f64::EPSILON * li * lj {
        let w = sub(j.a, i.a);
        let t = cross(w, dj) / c;
        let s = cross(w, di) / c;
        if t.is_finite() && s.is_finite() && interior(t, li) && interior(s, lj) {
            let p = [i.a[0] + t * di[0], i.a[1] + t * di[1]];
            on_i.push((t, p));
            on_j.push((s, p));
        }
    }
    Ok((on_i, on_j))
}

/// The representative of each point's cluster: the first of the points
/// that lie within `tol` of each other, chained. A cluster wider than
/// four tolerances is refused: the pattern is finer than the tolerance.
fn clusters(points: &[(u64, P2)], tol: f64) -> Result<Vec<usize>, FoldError> {
    let n = points.len();
    let mut parent = Union::new(n);
    let mut order: Vec<usize> = (0..n).collect();
    let x = |i: usize| points.get(i).map_or(0.0, |(_, p)| p[0]);
    order.sort_by(|&a, &b| x(a).total_cmp(&x(b)).then(a.cmp(&b)));
    for (k, &a) in order.iter().enumerate() {
        let Some(&(_, pa)) = points.get(a) else {
            continue;
        };
        for &b in order.iter().skip(k + 1) {
            let Some(&(_, pb)) = points.get(b) else {
                continue;
            };
            if pb[0] - pa[0] > tol {
                break;
            }
            if dist(pa, pb) <= tol {
                parent.join(a, b);
            }
        }
    }
    let mut rep = Vec::with_capacity(n);
    for i in 0..n {
        rep.push(parent.find(i));
    }
    for (i, &r) in rep.iter().enumerate() {
        if let (Some(&(s, p)), Some(&(_, q))) = (points.get(i), points.get(r))
            && dist(p, q) > 4.0 * tol
        {
            return Err(FoldError::Crowded { id: s });
        }
    }
    Ok(rep)
}

/// The sheet's loops as polygons (their segments head to tail).
fn boundary_rings(boundary: &[BoundarySegment]) -> Vec<Vec<P2>> {
    let mut rings: Vec<Vec<P2>> = Vec::new();
    let mut ring: Vec<P2> = Vec::new();
    let mut first: Option<P2> = None;
    for b in boundary {
        if first.is_none() {
            first = Some(b.start);
        }
        ring.push(b.start);
        if first.is_some_and(|f| f == b.end) {
            rings.push(std::mem::take(&mut ring));
            first = None;
        }
    }
    if !ring.is_empty() {
        rings.push(ring);
    }
    rings
}

pub(crate) fn signed_area(ring: &[P2]) -> f64 {
    let n = ring.len();
    let Some(&o) = ring.first() else { return 0.0 };
    let mut twice = 0.0;
    for i in 0..n {
        if let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) {
            twice += cross(sub(a, o), sub(b, o));
        }
    }
    twice / 2.0
}

fn perimeter(ring: &[P2]) -> f64 {
    let n = ring.len();
    (0..n)
        .filter_map(|i| Some(dist(*ring.get(i)?, *ring.get((i + 1) % n)?)))
        .sum()
}

/// Union-find whose representative is the smallest index of its set (so
/// a cluster is represented by its first point, deterministically).
struct Union(Vec<usize>);

impl Union {
    fn new(n: usize) -> Self {
        Union((0..n).collect())
    }

    fn find(&mut self, mut i: usize) -> usize {
        let mut steps = 0usize;
        while let Some(&p) = self.0.get(i) {
            if p == i || steps > self.0.len() {
                break;
            }
            let grand = self.0.get(p).copied().unwrap_or(p);
            if let Some(slot) = self.0.get_mut(i) {
                *slot = grand;
            }
            i = p;
            steps += 1;
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb
            && let Some(slot) = self.0.get_mut(ra.max(rb))
        {
            *slot = ra.min(rb);
        }
    }
}
