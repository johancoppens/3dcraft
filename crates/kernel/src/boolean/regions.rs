//! The pieces a face falls into when curves are imprinted on it.
//!
//! The boundary pieces of the face (each used one way) and the imprinted
//! pieces (each used both ways) form a map on the face's surface. Around
//! every vertex the pieces that leave it are sorted by direction in the
//! tangent plane; a region's boundary is followed by always taking, at the
//! end of a piece, the next piece clockwise from the way back, which keeps
//! the region on the left. Every use of a piece ends up in exactly one
//! such loop.
//!
//! Pieces that leave a vertex in the same direction (a circle tangent to
//! an edge it meets) are told apart a little further along: by the
//! directions to the points where they are all one small distance from the
//! vertex.
//!
//! An imprinted piece with the same region on both sides divides nothing
//! (the other body only touches the face along it) and is dropped.
//!
//! Loops are then grouped into regions in a chart of the surface
//! ([`super::chart`]): a loop that goes around a point just to its left is
//! the outside of a region, any other loop a hole in the innermost region
//! around it. On a sphere or a torus the holes that no loop is around
//! bound one more region, the one that contains the place the chart leaves
//! out.

use super::chart::{About, Chart, TOO_CLOSE};
use super::outline::{Seg, left_of, outward_normal};
use super::{Arc, Reason};
use craft3d_geom::Surface;
use craft3d_math::{Frame, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// The use of an arc: the arc's index and whether it is run backwards.
pub(super) type Half = (usize, bool);

/// Directions closer together than this (radians) at a vertex cannot be
/// put in order reliably.
const ANGLE_GUARD: f64 = 1e-6;

const TANGENT: Reason = "two curves leave a point in the same direction and stay together (tangent contact of higher order)";
const LOST: Reason = "an imprinted face could not be divided into regions; this is a bug";
const UNBOUNDED: Reason = "a piece of a face is not closed off by its boundary";

fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The loop pieces of a chain of arc uses.
pub(super) fn segs(loop_: &[Half], arcs: &[Arc], points: &[Vec3]) -> Result<Vec<Seg>, Reason> {
    let mut out = Vec::with_capacity(loop_.len());
    for &(arc, reversed) in loop_ {
        let arc = arcs.get(arc).ok_or(LOST)?;
        let (Some(&a), Some(&b)) = (points.get(arc.start), points.get(arc.end)) else {
            return Err(LOST);
        };
        out.push(Seg {
            curve: arc.curve.clone(),
            range: arc.range,
            a,
            b,
            reversed,
        });
    }
    Ok(out)
}

/// One end of an arc at its vertex.
struct Ray {
    vertex: usize,
    arc: usize,
    at_end: bool,
    /// The direction in which the arc leaves the vertex.
    direction: Vec3,
    angle: f64,
    /// The use of the arc that leaves along it, as an index into the uses.
    out: Option<usize>,
}

/// The point of the arc a fraction `along` of its range from the given end.
fn from_end(arc: &Arc, at_end: bool, along: f64) -> Vec3 {
    let (t0, t1) = arc.range;
    let t = if at_end {
        t1 - (t1 - t0) * along
    } else {
        t0 + (t1 - t0) * along
    };
    arc.curve.eval(t)
}

/// Puts rays that leave `origin` in the same direction in order: by the
/// direction to the point where each is at one small distance from the
/// vertex, which is the order in which they leave it as long as they do
/// not cross on the way (and arcs do not cross between vertices).
fn untie(
    run: &mut [usize],
    rays: &[Ray],
    arcs: &[Arc],
    origin: Vec3,
    frame: &Frame,
    tol: f64,
) -> Result<(), Reason> {
    let mut distance = f64::INFINITY;
    for &ray in run.iter() {
        let ray = rays.get(ray).ok_or(LOST)?;
        let arc = arcs.get(ray.arc).ok_or(LOST)?;
        distance = distance.min(0.125 * from_end(arc, ray.at_end, 0.25).distance(origin));
    }
    if !(distance.is_finite() && distance > 16.0 * tol) {
        return Err(TANGENT);
    }
    let base = rays
        .get(*run.first().ok_or(LOST)?)
        .map(|ray| (ray.direction.dot(frame.x()), ray.direction.dot(frame.y())))
        .ok_or(LOST)?;
    let mut keyed = Vec::with_capacity(run.len());
    for &index in run.iter() {
        let ray = rays.get(index).ok_or(LOST)?;
        let arc = arcs.get(ray.arc).ok_or(LOST)?;
        // Where along the first quarter the arc is that far away.
        let (mut near, mut far) = (0.0, 0.25);
        for _ in 0..60 {
            let mid = 0.5 * (near + far);
            if from_end(arc, ray.at_end, mid).distance(origin) < distance {
                near = mid;
            } else {
                far = mid;
            }
        }
        let chord = from_end(arc, ray.at_end, 0.5 * (near + far)) - origin;
        let (x, y) = (chord.dot(frame.x()), chord.dot(frame.y()));
        let offset = (base.0 * y - base.1 * x).atan2(base.0 * x + base.1 * y);
        if !offset.is_finite() {
            return Err(TANGENT);
        }
        keyed.push((offset, index));
    }
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    for pair in keyed.windows(2) {
        if let [a, b] = pair
            && b.0 - a.0 < ANGLE_GUARD
        {
            return Err(TANGENT);
        }
    }
    for (slot, (_, index)) in run.iter_mut().zip(keyed) {
        *slot = index;
    }
    Ok(())
}

/// The loops of the map: every use of an arc in exactly one.
fn trace(
    surface: &Surface,
    reversed: bool,
    boundary: &[Vec<Half>],
    interior: &BTreeSet<usize>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<Vec<Vec<Half>>, Reason> {
    let mut halves: Vec<Half> = boundary.iter().flatten().copied().collect();
    for &arc in interior {
        halves.push((arc, false));
        halves.push((arc, true));
    }

    // The ends of the arcs: a use leaves from one and arrives at the other.
    let mut rays: Vec<Ray> = Vec::new();
    let mut ray_at: BTreeMap<(usize, bool), usize> = BTreeMap::new();
    for (index, &(arc_index, backwards)) in halves.iter().enumerate() {
        let arc = arcs.get(arc_index).ok_or(LOST)?;
        for at_end in [false, true] {
            let key = (arc_index, at_end);
            let ray = match ray_at.get(&key) {
                Some(&ray) => ray,
                None => {
                    let (vertex, direction) = if at_end {
                        (arc.end, -arc.curve.derivatives(arc.range.1).1)
                    } else {
                        (arc.start, arc.curve.derivatives(arc.range.0).1)
                    };
                    rays.push(Ray {
                        vertex,
                        arc: arc_index,
                        at_end,
                        direction: direction.normalized().ok_or(LOST)?,
                        angle: 0.0,
                        out: None,
                    });
                    ray_at.insert(key, rays.len() - 1);
                    rays.len() - 1
                }
            };
            // Run backwards, a use leaves from the arc's end.
            if at_end == backwards {
                let slot = rays.get_mut(ray).ok_or(LOST)?;
                if slot.out.replace(index).is_some() {
                    return Err(LOST);
                }
            }
        }
    }

    // Per vertex its rays, counter-clockwise seen from outside.
    let mut fans: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, ray) in rays.iter().enumerate() {
        fans.entry(ray.vertex).or_default().push(index);
    }
    let mut place: Vec<(usize, usize)> = vec![(0, 0); rays.len()];
    for (&vertex, fan) in &mut fans {
        let point = *points.get(vertex).ok_or(LOST)?;
        let outward = outward_normal(surface, reversed, point).ok_or(LOST)?;
        let frame = Frame::from_z(point, outward).ok_or(LOST)?;
        for &index in fan.iter() {
            let ray = rays.get_mut(index).ok_or(LOST)?;
            ray.angle = ray
                .direction
                .dot(frame.y())
                .atan2(ray.direction.dot(frame.x()));
        }
        let angle = |index: usize| rays.get(index).map_or(0.0, |ray| ray.angle);
        fan.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)).then(a.cmp(&b)));
        let count = fan.len();
        if count > 1 {
            // Whether each ray leaves in the direction of the next one.
            let tied: Vec<bool> = (0..count)
                .map(|i| {
                    let here = fan.get(i).map_or(0.0, |&r| angle(r));
                    let next = fan.get((i + 1) % count).map_or(0.0, |&r| angle(r));
                    let gap = wrap(next - here);
                    gap < ANGLE_GUARD || gap > TAU - ANGLE_GUARD
                })
                .collect();
            if tied.iter().any(|&t| t) {
                // Start the fan where a run of such rays starts.
                let start = (0..count)
                    .find(|&i| !tied.get((i + count - 1) % count).copied().unwrap_or(false))
                    .unwrap_or(0);
                fan.rotate_left(start);
                let tied: Vec<bool> = (0..count)
                    .map(|i| tied.get((i + start) % count).copied().unwrap_or(false))
                    .collect();
                let mut from = 0;
                while from < count {
                    let mut to = from;
                    while to + 1 < count && tied.get(to).copied().unwrap_or(false) {
                        to += 1;
                    }
                    if to > from {
                        let run = fan.get_mut(from..=to).ok_or(LOST)?;
                        untie(run, &rays, arcs, point, &frame, tol)?;
                    }
                    from = to + 1;
                }
            }
        }
        for (position, &index) in fan.iter().enumerate() {
            if let Some(slot) = place.get_mut(index) {
                *slot = (vertex, position);
            }
        }
    }

    // The use that follows each use around its region: the first that
    // leaves clockwise from the way back; the way back itself where an
    // imprinted arc ends in the middle of the face.
    let mut following = Vec::with_capacity(halves.len());
    for &(arc, backwards) in &halves {
        let arrival = *ray_at.get(&(arc, !backwards)).ok_or(LOST)?;
        let &(vertex, position) = place.get(arrival).ok_or(LOST)?;
        let fan = fans.get(&vertex).ok_or(LOST)?;
        let count = fan.len();
        let mut next = None;
        for step in 1..=count {
            let ray = fan
                .get((position + count - step % count) % count)
                .and_then(|&r| rays.get(r))
                .ok_or(LOST)?;
            if let Some(out) = ray.out {
                next = Some(out);
                break;
            }
        }
        following.push(next.ok_or(LOST)?);
    }

    let mut loops: Vec<Vec<Half>> = Vec::new();
    let mut seen = vec![false; halves.len()];
    for start in 0..halves.len() {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut loop_ = Vec::new();
        let mut at = start;
        loop {
            let (Some(flag), Some(&half), Some(&next)) =
                (seen.get_mut(at), halves.get(at), following.get(at))
            else {
                return Err(LOST);
            };
            if *flag {
                return Err(LOST);
            }
            *flag = true;
            loop_.push(half);
            at = next;
            if at == start {
                break;
            }
            if loop_.len() > halves.len() {
                return Err(LOST);
            }
        }
        loops.push(loop_);
    }
    Ok(loops)
}

/// The regions of a face: each a list of loops, the outer boundary first
/// (when the region has one), then the holes.
pub(super) fn build(
    surface: &Surface,
    reversed: bool,
    boundary: &[Vec<Half>],
    interior: &BTreeSet<usize>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<Vec<Vec<Vec<Half>>>, Reason> {
    let mut interior = interior.clone();
    for _ in 0..=interior.len() {
        let loops = trace(surface, reversed, boundary, &interior, arcs, points, tol)?;
        // An imprinted arc that a loop runs along both ways has the same
        // region on both sides.
        let twice = |chains: &[Vec<Half>]| {
            let mut slits = BTreeSet::new();
            let mut used = BTreeSet::new();
            for half in chains.iter().flatten() {
                if !used.insert(half.0) && interior.contains(&half.0) {
                    slits.insert(half.0);
                }
            }
            slits
        };
        let mut slits = BTreeSet::new();
        for loop_ in &loops {
            slits.append(&mut twice(std::slice::from_ref(loop_)));
        }
        if slits.is_empty() {
            // So has one that two loops of one region run along: a closed
            // curve that does not divide its surface (a circle around a
            // torus).
            let regions = group(surface, reversed, loops, arcs, points, tol)?;
            for region in &regions {
                slits.append(&mut twice(region));
            }
            if slits.is_empty() {
                return Ok(regions);
            }
        }
        for slit in slits {
            interior.remove(&slit);
        }
    }
    Err(LOST)
}

struct Measured {
    halves: Vec<Half>,
    /// A point just inside the region the loop bounds.
    witness: Vec3,
    /// The loop goes around its region rather than around a hole in it.
    outer: bool,
}

fn group(
    surface: &Surface,
    reversed: bool,
    loops: Vec<Vec<Half>>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<Vec<Vec<Vec<Half>>>, Reason> {
    let mut all = Vec::with_capacity(loops.len());
    for halves in &loops {
        all.push(segs(halves, arcs, points)?);
    }
    let chart = Chart::new(surface, &all)?;
    if loops.is_empty() {
        // Nothing divides the whole of a closed surface.
        return if chart.is_closed() {
            Ok(vec![Vec::new()])
        } else {
            Err(UNBOUNDED)
        };
    }
    let around = |loop_: usize, about: About| -> Result<bool, Reason> {
        let segs = all.get(loop_).ok_or(LOST)?;
        match chart.winding([segs.as_slice()], about)? {
            0 => Ok(false),
            1 | -1 => Ok(true),
            _ => Err(TOO_CLOSE),
        }
    };
    let mut measured = Vec::with_capacity(loops.len());
    for (index, halves) in loops.into_iter().enumerate() {
        let own = all.get(index).ok_or(LOST)?;
        let witness = left_of(surface, reversed, &all, own, 4.0 * tol)?;
        measured.push(Measured {
            halves,
            witness,
            outer: around(index, About::Point(witness))?,
        });
    }

    // Of loops that are all around one point, the one inside the others.
    let innermost = |candidates: &[usize]| -> Result<Option<usize>, Reason> {
        for &candidate in candidates {
            let witness = measured.get(candidate).ok_or(LOST)?.witness;
            let mut inside_all = true;
            for &other in candidates {
                if other != candidate && !around(other, About::Point(witness))? {
                    inside_all = false;
                    break;
                }
            }
            if inside_all {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    };

    // Per outer loop its holes; and the holes that nothing is around.
    let mut holes: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut open: Vec<usize> = Vec::new();
    for (index, m) in measured.iter().enumerate() {
        if m.outer {
            holes.entry(index).or_default();
            continue;
        }
        let own: BTreeSet<usize> = m.halves.iter().map(|h| h.0).collect();
        let mut candidates = Vec::new();
        for (other, o) in measured.iter().enumerate() {
            // A loop that shares an arc with the hole has it from the
            // other side: it bounds what is in the hole, or beside it.
            if !o.outer || o.halves.iter().any(|h| own.contains(&h.0)) {
                continue;
            }
            if around(other, About::Point(m.witness))? {
                candidates.push(other);
            }
        }
        match innermost(&candidates)? {
            Some(home) => holes.entry(home).or_default().push(index),
            None if candidates.is_empty() => open.push(index),
            None => return Err(LOST),
        }
    }
    if !open.is_empty() {
        if !chart.is_closed() {
            return Err(UNBOUNDED);
        }
        // A torus is cut open along a circle that no loop crosses, so the
        // region at the inner end of the cut is the one at the outer end.
        let mut inner = None;
        if chart.has_inner_end() {
            let mut candidates = Vec::new();
            for (index, m) in measured.iter().enumerate() {
                if m.outer && around(index, About::InnerEnd)? {
                    candidates.push(index);
                }
            }
            inner = innermost(&candidates)?;
            if inner.is_none() && !candidates.is_empty() {
                return Err(LOST);
            }
        }
        if let Some(home) = inner {
            holes.entry(home).or_default().append(&mut open);
        }
    }

    let mut regions: Vec<Vec<usize>> = Vec::with_capacity(holes.len() + 1);
    for (outer, mut inside) in holes {
        inside.sort_unstable();
        let mut region = vec![outer];
        region.append(&mut inside);
        regions.push(region);
    }
    if !open.is_empty() {
        regions.push(open);
    }
    regions.sort_by(|a, b| a.first().cmp(&b.first()));
    let mut out = Vec::with_capacity(regions.len());
    for region in regions {
        let mut loops = Vec::with_capacity(region.len());
        for index in region {
            loops.push(measured.get(index).ok_or(LOST)?.halves.clone());
        }
        out.push(loops);
    }
    Ok(out)
}
