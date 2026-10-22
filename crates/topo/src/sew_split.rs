//! T-junctions for sewing (tolerant modelling, slice 3, 2026-10-22;
//! `docs/design/topology.md`, Method › *Sewing*): one free edge against
//! two or more shorter free edges along it, as a file from another
//! modeller has it where one face's side meets two faces of the other
//! side (a full circle against two half circles, a long side against the
//! sides of two faces split across it).
//!
//! **What is split.** A free edge `A` is split at *stations*: places
//! inside it (farther than the sewing distance from both its ends) where
//! the end of another free edge, of another face, lies within the
//! distance of `A`'s curve. Stations closer to each other than the
//! distance are one (the lowest parameter of them). The split is taken
//! only when the stations are *proven* a chain: between every two
//! neighbouring stations (and `A`'s ends) runs one free edge of another
//! face, its ends within the distance of the stations' points, its
//! places inside within the distance of `A`'s curve over that stretch and
//! the stretch's within the distance of it, its use running against
//! `A`'s; and no free edge runs along the whole of `A` (then it is a
//! pair, not a T-junction). An edge without such a chain is left whole.
//!
//! **Nothing moved.** A piece keeps `A`'s curve over a part of its range;
//! the vertex at a station is the point of `A`'s curve there, and sewing
//! then joins it to the ends of the shorter edges (the lowest vertex's
//! point is kept, which is one of the file's, never the new one); every
//! gap is measured and recorded as sewing measures any other.
//!
//! **Only when every piece is sewn.** The pieces are sewn with every
//! other free edge (`sew::pairs`); an edge one of whose pieces is left
//! open is left whole and the body sewn again without its split (at most
//! twice, then without any split), so a split never adds an open edge
//! that sewing alone would not have left.
//!
//! Bounded: the free edges' ends are kept sorted along x and looked up
//! by range, every end looked at counts against [`crate::sew`]'s probe
//! budget, and an edge with more than [`MAX_STATIONS`] stations is left
//! whole.

use crate::sew::{Free, SewError, inside, to_curve};
use crate::{Body, Coedge, EdgeId, VertexId};
use craft3d_geom::Curve3;
use craft3d_math::Vec3;
use std::collections::BTreeMap;

/// The most stations one edge is split at: more is left whole (a side
/// against a thousand facets is not what a file from a modeller has).
pub const MAX_STATIONS: usize = 1024;

/// Places along an edge from which the box it is looked for in is taken.
const BOX_SAMPLES: u32 = 32;

/// A free edge's end, sorted by `x`.
#[derive(Clone, Copy)]
struct End {
    x: f64,
    point: Vec3,
    edge: u32,
    /// Whether it is the edge's end (not its start).
    at_end: bool,
}

/// Per edge to split, the parameters inside its range at which it is
/// split, increasing.
pub(crate) type Stations = BTreeMap<u32, Vec<f64>>;

/// The free edges' ends, sorted along x.
struct Ends {
    sorted: Vec<End>,
}

impl Ends {
    /// The indices of the ends whose x lies within `[lo, hi]`.
    fn within_x(&self, lo: f64, hi: f64) -> std::ops::Range<usize> {
        let from = self.sorted.partition_point(|e| e.x < lo);
        let to = self.sorted.partition_point(|e| e.x <= hi);
        from..to.max(from)
    }
}

/// The parameter of `curve` within `range` nearest `p`, and how far it is,
/// when the nearest point lies inside the range.
fn param_on(curve: &Curve3, range: (f64, f64), p: Vec3) -> Option<(f64, f64)> {
    let (t0, t1) = range;
    let mut t = curve.project(p).ok()?;
    if let Some(period) = curve.period()
        && period > 0.0
    {
        let turns = ((t0 - t) / period).ceil();
        if turns.is_finite() && turns.abs() < 1e6 {
            t += turns * period;
        }
    }
    if !(t > t0 && t < t1) {
        return None;
    }
    let d = curve.eval(t).distance(p);
    d.is_finite().then_some((t, d))
}

/// Whether the edges `a` over `ra` and `b` over `rb` run within
/// `distance` of each other at the places inside each.
fn alongside(a: &Curve3, ra: (f64, f64), b: &Curve3, rb: (f64, f64), distance: f64) -> bool {
    inside(a, ra)
        .into_iter()
        .all(|p| to_curve(b, rb, p) <= distance)
        && inside(b, rb)
            .into_iter()
            .all(|p| to_curve(a, ra, p) <= distance)
}

/// The stations of every free edge that is one side of a T-junction (see
/// the module documentation).
pub(crate) fn stations(
    body: &Body,
    free: &BTreeMap<u32, Free>,
    distance: f64,
    probes: &mut usize,
) -> Result<Stations, SewError> {
    let mut out = Stations::new();
    if free.len() < 3 {
        return Ok(out);
    }
    let point = |v: VertexId| body.vertex(v).map(|v| v.point).ok_or(SewError::Dangling);
    let mut sorted = Vec::with_capacity(2 * free.len());
    for &e in free.keys() {
        let edge = body.edge(EdgeId(e)).ok_or(SewError::Dangling)?;
        for (v, at_end) in [(edge.start, false), (edge.end, true)] {
            let p = point(v)?;
            if p.x.is_finite() && p.y.is_finite() && p.z.is_finite() {
                sorted.push(End {
                    x: p.x,
                    point: p,
                    edge: e,
                    at_end,
                });
            }
        }
    }
    sorted.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.edge.cmp(&b.edge))
            .then(a.at_end.cmp(&b.at_end))
    });
    let ends = Ends { sorted };
    let mut count = |n: usize| -> Result<(), SewError> {
        *probes = probes.saturating_add(n);
        if *probes > crate::sew::MAX_PROBES {
            return Err(SewError::TooMany {
                candidates: crate::sew::MAX_PROBES,
            });
        }
        Ok(())
    };
    // The free edges whose ends lie within `distance` of `p`: (edge,
    // whether that end is its end).
    let near = |p: Vec3, count: &mut dyn FnMut(usize) -> Result<(), SewError>| {
        let range = ends.within_x(p.x - distance, p.x + distance);
        count(range.len())?;
        Ok::<Vec<(u32, bool)>, SewError>(
            ends.sorted
                .get(range)
                .unwrap_or(&[])
                .iter()
                .filter(|e| e.point.distance(p) <= distance)
                .map(|e| (e.edge, e.at_end))
                .collect(),
        )
    };
    for (&a, fa) in free {
        let edge = body.edge(EdgeId(a)).ok_or(SewError::Dangling)?;
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
            continue;
        }
        let [a_start, a_end] = [point(edge.start)?, point(edge.end)?];
        // The box the edge runs in, from places along it, widened by the
        // longest step between them and the distance.
        let samples: Vec<Vec3> = (0..=BOX_SAMPLES)
            .map(|k| {
                edge.curve
                    .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(BOX_SAMPLES))
            })
            .collect();
        if samples
            .iter()
            .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
        {
            continue;
        }
        let step = samples
            .iter()
            .zip(samples.iter().skip(1))
            .map(|(p, q)| p.distance(*q))
            .fold(0.0, f64::max);
        let reach = distance + step;
        let lo = samples.iter().fold(
            Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            |m, p| m.min(*p),
        );
        let hi = samples.iter().fold(
            Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
            |m, p| m.max(*p),
        );
        let range = ends.within_x(lo.x - reach, hi.x + reach);
        count(range.len())?;
        let mut found: Vec<(f64, u32)> = Vec::new();
        for end in ends.sorted.get(range).unwrap_or(&[]) {
            let p = end.point;
            if end.edge == a
                || p.y < lo.y - reach
                || p.y > hi.y + reach
                || p.z < lo.z - reach
                || p.z > hi.z + reach
                || p.distance(a_start) <= distance
                || p.distance(a_end) <= distance
                || free.get(&end.edge).is_none_or(|f| f.face == fa.face)
            {
                continue;
            }
            if let Some((s, d)) = param_on(&edge.curve, edge.range, p)
                && d <= distance
            {
                found.push((s, end.edge));
            }
        }
        if found.is_empty() {
            continue;
        }
        found.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        // One station per place.
        let mut cuts: Vec<f64> = Vec::new();
        for (s, _) in found {
            let apart = cuts
                .last()
                .is_none_or(|&last| edge.curve.eval(last).distance(edge.curve.eval(s)) > distance);
            if apart {
                cuts.push(s);
            }
        }
        if cuts.len() > MAX_STATIONS {
            continue;
        }
        // The stretches between the stations and the ends.
        let mut params = Vec::with_capacity(cuts.len() + 2);
        params.push(t0);
        params.extend(cuts.iter().copied());
        params.push(t1);
        let mut points = Vec::with_capacity(params.len());
        points.push(a_start);
        points.extend(cuts.iter().map(|&s| edge.curve.eval(s)));
        points.push(a_end);
        // A partner over the stretch from `p` to `q` (`r` of the curve):
        // a free edge of another face with its ends there, running
        // against `a`'s use, alongside it.
        let partner = |p: Vec3,
                       q: Vec3,
                       r: (f64, f64),
                       count: &mut dyn FnMut(usize) -> Result<(), SewError>|
         -> Result<Option<u32>, SewError> {
            let mut best: Option<u32> = None;
            for (b, at_end) in near(p, count)? {
                if b == a || best.is_some_and(|x| x <= b) {
                    continue;
                }
                let Some(fb) = free.get(&b) else {
                    continue;
                };
                if fb.face == fa.face {
                    continue;
                }
                let eb = body.edge(EdgeId(b)).ok_or(SewError::Dangling)?;
                // b's other end at q; b runs from p to q against its own
                // direction when its end is at p.
                let other = if at_end { eb.start } else { eb.end };
                if point(other)?.distance(q) > distance {
                    continue;
                }
                let flip = at_end;
                if (fb.reversed ^ flip) == fa.reversed {
                    continue;
                }
                if alongside(&edge.curve, r, &eb.curve, eb.range, distance) {
                    best = Some(b);
                }
            }
            Ok(best)
        };
        // A partner along the whole edge: a pair, not a T-junction.
        if partner(a_start, a_end, edge.range, &mut count)?.is_some() {
            continue;
        }
        let mut chain = true;
        let mut used: Vec<u32> = Vec::with_capacity(cuts.len() + 1);
        for i in 0..params.len().saturating_sub(1) {
            let (Some(&p), Some(&q), Some(&r0), Some(&r1)) = (
                points.get(i),
                points.get(i + 1),
                params.get(i),
                params.get(i + 1),
            ) else {
                chain = false;
                break;
            };
            match partner(p, q, (r0, r1), &mut count)? {
                Some(b) if !used.contains(&b) => used.push(b),
                _ => {
                    chain = false;
                    break;
                }
            }
        }
        if chain {
            out.insert(a, cuts);
        }
    }
    Ok(out)
}

/// `body` with each edge of `at` split at its stations (see the module
/// documentation), and per edge split its pieces in order (the first is
/// the edge itself, the others new edges after the body's own); the new
/// vertices come after the body's own.
pub(crate) fn split(
    body: &Body,
    at: &Stations,
) -> Result<(Body, BTreeMap<u32, Vec<EdgeId>>), SewError> {
    let mut out = body.clone();
    let mut pieces: BTreeMap<u32, Vec<EdgeId>> = BTreeMap::new();
    for (&a, cuts) in at {
        let edge = out.edge(EdgeId(a)).cloned().ok_or(SewError::Dangling)?;
        let own = out.edge_tolerance(EdgeId(a));
        let mut bounds = Vec::with_capacity(cuts.len() + 2);
        bounds.push(edge.range.0);
        bounds.extend(cuts.iter().copied());
        bounds.push(edge.range.1);
        let mut vertices = Vec::with_capacity(bounds.len());
        vertices.push(edge.start);
        for &s in cuts {
            vertices.push(out.add_vertex(edge.curve.eval(s)));
        }
        vertices.push(edge.end);
        let mut ids = Vec::with_capacity(cuts.len() + 1);
        for i in 0..bounds.len().saturating_sub(1) {
            let (Some(&r0), Some(&r1), Some(&v0), Some(&v1)) = (
                bounds.get(i),
                bounds.get(i + 1),
                vertices.get(i),
                vertices.get(i + 1),
            ) else {
                return Err(SewError::Dangling);
            };
            let id = if i == 0 {
                let slot = out.edges.get_mut(a as usize).ok_or(SewError::Dangling)?;
                slot.range = (r0, r1);
                slot.end = v1;
                EdgeId(a)
            } else {
                out.add_edge(edge.curve.clone(), (r0, r1), v0, v1)
            };
            if let Some(t) = own {
                out.set_edge_tolerance(id, t)
                    .map_err(|_| SewError::Dangling)?;
            }
            ids.push(id);
        }
        pieces.insert(a, ids);
    }
    for l in &mut out.loops {
        let mut coedges = Vec::with_capacity(l.coedges.len());
        for c in &l.coedges {
            match pieces.get(&c.edge.0) {
                None => coedges.push(*c),
                Some(ids) if c.reversed => coedges.extend(ids.iter().rev().map(|&edge| Coedge {
                    edge,
                    reversed: true,
                })),
                Some(ids) => coedges.extend(ids.iter().map(|&edge| Coedge {
                    edge,
                    reversed: false,
                })),
            }
        }
        l.coedges = coedges;
    }
    Ok((out, pieces))
}
