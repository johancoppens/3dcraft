//! Whether two facets of a folded sheet pass through each other or touch,
//! other than where the crease pattern joins them (the creases and the
//! vertices they share in the flat sheet; two facets on either side of a
//! cut share its vertices and its edge too).
//!
//! Two planar facets in planes that cross meet only on the planes' common
//! line `L`: each facet's part of `L` is a set of intervals (where its
//! boundary crosses `L`, and between those points where its middle lies in
//! the facet), and the facets meet where their intervals overlap. What the
//! crease pattern shares is taken out (the shared edge's stretch of `L`,
//! a tolerance round each shared vertex); anything left is a contact,
//! *through* where the middle of the overlap lies inside both facets
//! (further than the tolerance from their edges), *touch* otherwise. Two
//! facets in one plane (their normals within the angular tolerance and
//! every corner within the tolerance of the other's plane) meet when a
//! corner of one that is not shared lies in the other (inside or within
//! the tolerance of its edges), or when their edges cross away from the
//! shared vertices: a fold of 180 degrees lays two facets on one another,
//! which a sheet of no thickness cannot be, so it is a *touch*.

use super::{Arrangement, FoldError, Folded, P2, inside_loops, lift, ring_distance};
use crate::boxtree::BoxTree;
use craft3d_math::{Bbox3, Transform, Vec3};
use std::collections::BTreeSet;

/// How two facets meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetMeeting {
    /// They pass through each other.
    Through,
    /// They touch: along an edge, at a point, or lying on each other.
    Touch,
}

impl FacetMeeting {
    pub fn verb(self) -> &'static str {
        match self {
            FacetMeeting::Through => "pass through each other",
            FacetMeeting::Touch => "touch or lie on each other",
        }
    }
}

/// Two facets that meet, and a point where they do (sheet coordinates,
/// folded).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meeting {
    pub facets: [usize; 2],
    pub kind: FacetMeeting,
    pub at: Vec3,
}

pub(super) struct Placed {
    /// The loops in the flat sheet.
    flat: Vec<Vec<P2>>,
    /// The same loops folded.
    folded: Vec<Vec<Vec3>>,
    /// The arrangement vertex of each corner, loop by loop.
    ids: Vec<Vec<usize>>,
    /// The arrangement edge from each corner to the next, loop by loop.
    edge_ids: Vec<Vec<usize>>,
    vertices: BTreeSet<usize>,
    edges: BTreeSet<usize>,
    normal: Vec3,
    origin: Vec3,
    lo: Vec3,
    hi: Vec3,
    /// From flat to folded, and back.
    forth: Transform,
    back: Transform,
}

fn placed(arr: &Arrangement, folded: &Folded, f: usize) -> Option<Placed> {
    let facet = arr.facets.get(f)?;
    let t = folded.transforms.get(f)?;
    let mut flat = Vec::new();
    let mut moved = Vec::new();
    let mut ids = Vec::new();
    let mut edge_ids = Vec::new();
    let mut vertices = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    for cycle in std::iter::once(&facet.outer).chain(&facet.holes) {
        let mut ring = Vec::with_capacity(cycle.len());
        let mut ring3 = Vec::with_capacity(cycle.len());
        let mut ring_ids = Vec::with_capacity(cycle.len());
        let mut ring_edges = Vec::with_capacity(cycle.len());
        for &h in cycle {
            let v = arr.tail(h)?;
            let p = arr.point(v)?;
            let q = t.point(lift(p));
            lo = lo.min(q);
            hi = hi.max(q);
            ring.push(p);
            ring3.push(q);
            ring_ids.push(v);
            ring_edges.push(h / 2);
            vertices.insert(v);
            edges.insert(h / 2);
        }
        flat.push(ring);
        moved.push(ring3);
        ids.push(ring_ids);
        edge_ids.push(ring_edges);
    }
    let origin = *moved.first()?.first()?;
    Some(Placed {
        flat,
        folded: moved,
        ids,
        edge_ids,
        vertices,
        edges,
        normal: t.vector(Vec3::Z),
        origin,
        lo,
        hi,
        forth: *t,
        back: t.inverse()?,
    })
}

impl Placed {
    /// Distance from a flat point to the facet's edges.
    fn edge_distance(&self, p: P2) -> f64 {
        ring_distance(&self.flat, p)
    }

    fn inside_flat(&self, p: P2) -> bool {
        inside_loops(p, self.flat.iter().map(Vec::as_slice))
    }

    /// Whether the folded point `x` (on the facet's plane) lies in the
    /// closed facet, and whether in its interior.
    fn holds(&self, x: Vec3, tol: f64) -> (bool, bool) {
        let q = self.back.point(x);
        let p = [q.x, q.y];
        let d = self.edge_distance(p);
        let inside = self.inside_flat(p);
        (inside || d <= tol, inside && d > tol)
    }
}

/// The first pair of facets (in the order of their indices) that pass
/// through each other or touch, if any. Fails closed: a facet that cannot
/// be placed is an error, never "no meeting". The pairs looked at are those
/// whose boxes (grown by the tolerance) meet, found by a box tree, so the
/// work grows with the facets and the pairs that are near, not with every
/// pair.
pub fn first_meeting(
    arr: &Arrangement,
    folded: &Folded,
    tol: f64,
    angular: f64,
) -> Result<Option<Meeting>, FoldError> {
    let mut all = Vec::with_capacity(arr.facets.len());
    for f in 0..arr.facets.len() {
        all.push(placed(arr, folded, f).ok_or(FoldError::Broken {
            reason: "a facet could not be placed to look for meetings",
        })?);
    }
    let grow = Vec3::new(tol, tol, tol);
    let boxes: Vec<Bbox3> = all
        .iter()
        .map(|p| Bbox3::from_points([p.lo - grow, p.hi + grow]))
        .collect();
    let tree = BoxTree::new(boxes.clone());
    let mut near = Vec::new();
    for (i, p) in all.iter().enumerate() {
        let Some(mine) = boxes.get(i) else { continue };
        let mut budget = usize::MAX;
        tree.search(|b| boxes_meet(b, mine), &mut near, &mut budget);
        near.retain(|&j| j > i);
        near.sort_unstable();
        for &j in &near {
            let Some(q) = all.get(j) else { continue };
            if let Some((kind, at)) = meet(p, q, tol, angular) {
                return Ok(Some(Meeting {
                    facets: [i, j],
                    kind,
                    at,
                }));
            }
        }
    }
    Ok(None)
}

fn boxes_meet(a: &Bbox3, b: &Bbox3) -> bool {
    match (a.corners(), b.corners()) {
        (Some((alo, ahi)), Some((blo, bhi))) => {
            alo.x <= bhi.x
                && blo.x <= ahi.x
                && alo.y <= bhi.y
                && blo.y <= ahi.y
                && alo.z <= bhi.z
                && blo.z <= ahi.z
        }
        _ => false,
    }
}

pub(super) fn meet(p: &Placed, q: &Placed, tol: f64, angular: f64) -> Option<(FacetMeeting, Vec3)> {
    let shared_v: BTreeSet<usize> = p.vertices.intersection(&q.vertices).copied().collect();
    let shared_e: BTreeSet<usize> = p.edges.intersection(&q.edges).copied().collect();
    // Parallel when the normals are within the angular tolerance (the
    // sine of the angle between them).
    let parallel = p.normal.cross(q.normal).length() <= angular;
    if parallel {
        let off = |x: Vec3| (x - p.origin).dot(p.normal).abs();
        if q.folded.iter().flatten().any(|&x| off(x) > tol) {
            return None;
        }
        return coplanar(p, q, &shared_v, &shared_e, tol);
    }
    let cross = p.normal.cross(q.normal);
    let u = cross.normalized()?;
    // A point on both planes: n_p·x = d_p, n_q·x = d_q.
    let (dp, dq) = (p.normal.dot(p.origin), q.normal.dot(q.origin));
    let x0 =
        (q.normal.cross(cross) * dp + cross.cross(p.normal) * dq) * (1.0 / cross.length_squared());
    if !x0.is_finite() {
        return None;
    }
    let ip = on_line(p, x0, u, tol);
    let iq = on_line(q, x0, u, tol);
    let mut pieces = Vec::new();
    for &(a0, a1) in &ip {
        for &(b0, b1) in &iq {
            let (lo, hi) = (a0.max(b0), a1.min(b1));
            if lo <= hi + tol {
                pieces.push((lo.min(hi), hi.max(lo)));
            }
        }
    }
    if pieces.is_empty() {
        return None;
    }
    // What the pattern shares, taken out.
    let mut removed: Vec<(f64, f64)> = Vec::new();
    let t_of = |x: Vec3| (x - x0).dot(u);
    let off_line = |x: Vec3| {
        let d = x - x0;
        (d - u * d.dot(u)).length()
    };
    for ((ring, ids), edge_ids) in p.folded.iter().zip(&p.ids).zip(&p.edge_ids) {
        let n = ring.len();
        for k in 0..n {
            let (Some(&a), Some(&va)) = (ring.get(k), ids.get(k)) else {
                continue;
            };
            if shared_v.contains(&va) && off_line(a) <= tol {
                let t = t_of(a);
                removed.push((t - tol, t + tol));
            }
            let Some(&b) = ring.get((k + 1) % n) else {
                continue;
            };
            let edge_shared = edge_ids.get(k).is_some_and(|e| shared_e.contains(e))
                && off_line(a) <= tol
                && off_line(b) <= tol;
            if edge_shared {
                let (ta, tb) = (t_of(a), t_of(b));
                removed.push((ta.min(tb) - tol, ta.max(tb) + tol));
            }
        }
    }
    let left = subtract(&pieces, &removed);
    let &(lo, hi) = left.first()?;
    let mid = x0 + u * ((lo + hi) / 2.0);
    let (_, inner_p) = p.holds(mid, tol);
    let (_, inner_q) = q.holds(mid, tol);
    let kind = if inner_p && inner_q && hi - lo > tol {
        FacetMeeting::Through
    } else {
        FacetMeeting::Touch
    };
    Some((kind, mid))
}

/// The closed set of the facet on the line `x0 + t u` (which lies in its
/// plane), as intervals of `t`, ascending; a point is an interval of no
/// length.
fn on_line(f: &Placed, x0: Vec3, u: Vec3, tol: f64) -> Vec<(f64, f64)> {
    let w = f.normal.cross(u);
    let mut ts: Vec<f64> = Vec::new();
    for ring in &f.folded {
        let n = ring.len();
        for k in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(k), ring.get((k + 1) % n)) else {
                continue;
            };
            let (sa, sb) = ((a - x0).dot(w), (b - x0).dot(w));
            if sa.abs() <= tol {
                ts.push((a - x0).dot(u));
            }
            if (sa > tol && sb < -tol) || (sa < -tol && sb > tol) {
                let s = sa / (sa - sb);
                let x = a + (b - a) * s;
                ts.push((x - x0).dot(u));
            }
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|b, a| (*b - *a).abs() <= tol * 1e-3);
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut push = |lo: f64, hi: f64| match out.last_mut() {
        Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
        _ => out.push((lo, hi)),
    };
    for (k, &t) in ts.iter().enumerate() {
        push(t, t);
        if let Some(&next) = ts.get(k + 1) {
            let mid = x0 + u * ((t + next) / 2.0);
            if f.holds(mid, tol).0 {
                push(t, next);
            }
        }
    }
    out
}

/// `pieces` with the intervals of `removed` taken out (open removals: a
/// piece's part strictly inside one goes).
fn subtract(pieces: &[(f64, f64)], removed: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut left: Vec<(f64, f64)> = pieces.to_vec();
    for &(r0, r1) in removed {
        let mut next = Vec::with_capacity(left.len());
        for (a, b) in left {
            if b < r0 || a > r1 {
                next.push((a, b));
                continue;
            }
            if a < r0 {
                next.push((a, r0));
            }
            if b > r1 {
                next.push((r1, b));
            }
        }
        left = next;
    }
    left
}

fn coplanar(
    p: &Placed,
    q: &Placed,
    shared_v: &BTreeSet<usize>,
    shared_e: &BTreeSet<usize>,
    tol: f64,
) -> Option<(FacetMeeting, Vec3)> {
    // Everything in the flat coordinates of `p`.
    let to_p = |x: Vec3| {
        let y = p.back.point(x);
        [y.x, y.y]
    };
    let q_flat: Vec<Vec<P2>> = q
        .folded
        .iter()
        .map(|r| r.iter().map(|&x| to_p(x)).collect())
        .collect();
    let q_dist = |pt: P2| ring_distance(&q_flat, pt);
    let q_inside = |pt: P2| inside_loops(pt, q_flat.iter().map(Vec::as_slice));
    let p_point = |pt: P2| p.folded_of(pt);
    // A corner of q, not shared, in p.
    for (ring, ids) in q.folded.iter().zip(&q.ids) {
        for (&x, v) in ring.iter().zip(ids) {
            if shared_v.contains(v) {
                continue;
            }
            let pt = to_p(x);
            if p.inside_flat(pt) || p.edge_distance(pt) <= tol {
                return Some((FacetMeeting::Touch, x));
            }
        }
    }
    // A corner of p, not shared, in q.
    for (ring, ids) in p.flat.iter().zip(&p.ids) {
        for (&pt, v) in ring.iter().zip(ids) {
            if shared_v.contains(v) {
                continue;
            }
            if q_inside(pt) || q_dist(pt) <= tol {
                return Some((FacetMeeting::Touch, p_point(pt)));
            }
        }
    }
    // Edges that cross away from the shared vertices.
    let shared_points: Vec<P2> = p
        .flat
        .iter()
        .zip(&p.ids)
        .flat_map(|(r, ids)| r.iter().zip(ids))
        .filter(|(_, v)| shared_v.contains(v))
        .map(|(pt, _)| *pt)
        .collect();
    for (ring, pe) in p.flat.iter().zip(&p.edge_ids) {
        let n = ring.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                continue;
            };
            for (qr, qe) in q_flat.iter().zip(&q.edge_ids) {
                let m = qr.len();
                for j in 0..m {
                    let (Some(&c), Some(&d)) = (qr.get(j), qr.get((j + 1) % m)) else {
                        continue;
                    };
                    if pe.get(i).is_some_and(|e| shared_e.contains(e)) && pe.get(i) == qe.get(j) {
                        continue;
                    }
                    if let Some(x) = crossing(a, b, c, d)
                        && shared_points.iter().all(|s| super::dist(*s, x) > tol)
                    {
                        return Some((FacetMeeting::Touch, p_point(x)));
                    }
                }
            }
        }
    }
    None
}

impl Placed {
    fn folded_of(&self, p: P2) -> Vec3 {
        self.forth.point(lift(p))
    }

    /// Every corner, placed.
    pub(super) fn corners(&self) -> Vec<Vec3> {
        self.folded.iter().flatten().copied().collect()
    }
}

/// A planar polygon (`rings` in its own plane's coordinates, z = 0)
/// placed by `forth`, sharing nothing with any other: every corner and
/// edge numbered afresh from `next_id`. What the contact test of two
/// facets asks of the faces of thick panels.
pub(super) fn polygon_face(
    rings: Vec<Vec<P2>>,
    forth: Transform,
    next_id: &mut usize,
) -> Option<Placed> {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    let mut folded = Vec::with_capacity(rings.len());
    let mut ids = Vec::with_capacity(rings.len());
    let mut edge_ids = Vec::with_capacity(rings.len());
    let mut vertices = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for ring in &rings {
        let mut moved = Vec::with_capacity(ring.len());
        let mut r_ids = Vec::with_capacity(ring.len());
        let mut r_edges = Vec::with_capacity(ring.len());
        for &p in ring {
            let q = forth.point(lift(p));
            lo = lo.min(q);
            hi = hi.max(q);
            moved.push(q);
            let id = *next_id;
            *next_id = next_id.saturating_add(1);
            r_ids.push(id);
            r_edges.push(id);
            vertices.insert(id);
            edges.insert(id);
        }
        folded.push(moved);
        ids.push(r_ids);
        edge_ids.push(r_edges);
    }
    let origin = *folded.first()?.first()?;
    Some(Placed {
        flat: rings,
        folded,
        ids,
        edge_ids,
        vertices,
        edges,
        normal: forth.vector(Vec3::Z),
        origin,
        lo,
        hi,
        forth,
        back: forth.inverse()?,
    })
}

/// Where segments `a`–`b` and `c`–`d` cross properly.
fn crossing(a: P2, b: P2, c: P2, d: P2) -> Option<P2> {
    let r = super::sub(b, a);
    let s = super::sub(d, c);
    let den = super::cross(r, s);
    if den == 0.0 {
        return None;
    }
    let w = super::sub(c, a);
    let t = super::cross(w, s) / den;
    let v = super::cross(w, r) / den;
    (t > 0.0 && t < 1.0 && v > 0.0 && v < 1.0).then(|| [a[0] + t * r[0], a[1] + t * r[1]])
}
