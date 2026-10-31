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
use super::outline::{Seg, Sheet, boxes, left_of, outward_normal};
use super::{Arc, Reason};
use craft3d_geom::Surface;
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// The use of an arc: the arc's index and whether it is run backwards.
pub(super) type Half = (usize, bool);

/// Directions closer together than this (radians) at a vertex cannot be
/// put in order reliably.
pub(super) const ANGLE_GUARD: f64 = 1e-6;

/// Directions closer together than this (radians) at a vertex are not
/// trusted to be in the right order as they stand: an intersection curve
/// is only within the tolerance of the curve it stands for, and where it
/// arrives along another curve (the curve of two cylinders that touches a
/// rim at its highest point) its direction at its very end may be off to
/// the wrong side by more than `ANGLE_GUARD`. Such curves are put in order
/// a little further along, where the tolerance no longer decides.
const TIE: f64 = 1e-3;

/// How many tolerances apart two arcs must be, at the distance from a
/// vertex where their order is read, for the order to be theirs and not
/// that of the curves' approximation.
const APART: f64 = 4.0;

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
///
/// An arc is only within the tolerance of the curve it stands for, so
/// the order means something only where the arcs are further apart than
/// that: at a distance where two of them are still within a few
/// tolerances of each other the next distance is tried, twice as far, up
/// to a quarter of the shortest arc. (The curve of a hole that touches a
/// rim where the rim meets a tangent wall leaves the rim with contact of
/// the fourth order: a sixtieth of the way along they are a hundredth of
/// a tolerance apart, a quarter of the way forty tolerances.)
fn untie(
    run: &mut [usize],
    rays: &[Ray],
    arcs: &[Arc],
    origin: Vec3,
    frame: &Frame,
    tol: f64,
) -> Result<(), Reason> {
    let mut nearest = f64::INFINITY;
    for &ray in run.iter() {
        let ray = rays.get(ray).ok_or(LOST)?;
        let arc = arcs.get(ray.arc).ok_or(LOST)?;
        nearest = nearest.min(from_end(arc, ray.at_end, 0.25).distance(origin));
    }
    let base = rays
        .get(*run.first().ok_or(LOST)?)
        .map(|ray| (ray.direction.dot(frame.x()), ray.direction.dot(frame.y())))
        .ok_or(LOST)?;
    // (Along the normal, at a cusp, the first ray has no direction across
    // it: any fixed one serves to read the others from.)
    let base = if base.0.phypot(base.1) < 1e-9 {
        (1.0, 0.0)
    } else {
        base
    };
    for part in [0.125, 0.25, 0.5, 1.0] {
        let distance = part * nearest;
        if !(distance.is_finite() && distance > 16.0 * tol) {
            return Err(TANGENT);
        }
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
            let offset = (base.0 * y - base.1 * x).patan2(base.0 * x + base.1 * y);
            if !offset.is_finite() {
                return Err(TANGENT);
            }
            keyed.push((offset, index));
        }
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // Apart by more than the tolerance can account for, there.
        let guard = ANGLE_GUARD.max(APART * tol / distance);
        let clear = keyed
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if b.0 - a.0 >= guard));
        if clear {
            for (slot, (_, index)) in run.iter_mut().zip(keyed) {
                *slot = index;
            }
            return Ok(());
        }
    }
    Err(TANGENT)
}

/// Which arcs of a face's map are imprinted inside it, and which the
/// boundary uses exactly once.
struct Sides<'a> {
    interior: &'a BTreeSet<usize>,
    once: BTreeSet<usize>,
}

/// Puts a run of rays that leave a vertex in the same direction in order.
///
/// Where the run holds one piece of the face's boundary and pieces
/// imprinted on the face, the order between the boundary and the rest is
/// known without measuring: an imprinted piece lies in the face (it was
/// clipped to it, and arcs do not cross between vertices), and the face
/// lies to the left of its boundary, counter-clockwise from the use that
/// leaves the vertex and clockwise from the one that arrives. So the
/// imprinted pieces follow the boundary piece that leaves and precede the
/// one that arrives, whatever the surfaces and however shallow the angle
/// at which they meet; only their order among themselves is measured
/// ([`untie`]). (A curve that crosses the boundary at a millionth of a
/// radian near another vertex of it, as where a cone's base circle lies a
/// few hundred tolerances off a cylinder's wall, is within the tolerance
/// of the boundary further than that vertex: no distance along tells them
/// apart.) A boundary piece the boundary uses twice (a seam, with the face
/// on both sides of it) says nothing of the side, nor do two boundary
/// pieces in one run (a corner of no angle); any such run is measured as
/// a whole.
fn order_run(
    run: &mut [usize],
    rays: &[Ray],
    arcs: &[Arc],
    sides: &Sides,
    origin: Vec3,
    frame: &Frame,
    tol: f64,
) -> Result<(), Reason> {
    let mut edge = Vec::with_capacity(1);
    let mut inner = Vec::with_capacity(run.len());
    for &index in run.iter() {
        let ray = rays.get(index).ok_or(LOST)?;
        if sides.interior.contains(&ray.arc) {
            inner.push(index);
        } else {
            edge.push(index);
        }
    }
    let [edge] = edge.as_slice() else {
        return untie(run, rays, arcs, origin, frame, tol);
    };
    if !sides.once.contains(&rays.get(*edge).ok_or(LOST)?.arc) {
        return untie(run, rays, arcs, origin, frame, tol);
    }
    if inner.len() > 1 {
        untie(&mut inner, rays, arcs, origin, frame, tol)?;
    }
    let leaves = rays.get(*edge).ok_or(LOST)?.out.is_some();
    let ordered: Vec<usize> = if leaves {
        std::iter::once(*edge).chain(inner).collect()
    } else {
        inner.into_iter().chain(std::iter::once(*edge)).collect()
    };
    for (slot, index) in run.iter_mut().zip(ordered) {
        *slot = index;
    }
    Ok(())
}

/// The loops of the map: every use of an arc in exactly one.
fn trace(
    surface: Sheet,
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
    let mut uses: BTreeMap<usize, usize> = BTreeMap::new();
    for &(arc, _) in boundary.iter().flatten() {
        *uses.entry(arc).or_default() += 1;
    }
    let sides = Sides {
        interior,
        once: uses
            .into_iter()
            .filter_map(|(arc, count)| (count == 1).then_some(arc))
            .collect(),
    };

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
                .patan2(ray.direction.dot(frame.x()));
        }
        let angle = |index: usize| rays.get(index).map_or(0.0, |ray| ray.angle);
        fan.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)).then(a.cmp(&b)));
        let count = fan.len();
        // At a cusp of the surface every piece leaves the point along the
        // line it closes around, square to the tangent plane the angles
        // are read in: they say nothing, and the pieces are put in order
        // a little further along, all of them.
        let axial = fan.iter().all(|&index| {
            rays.get(index).is_some_and(|ray| {
                let (x, y) = (ray.direction.dot(frame.x()), ray.direction.dot(frame.y()));
                x.phypot(y) < TIE
            })
        });
        if count > 1 && axial {
            untie(fan, &rays, arcs, point, &frame, tol)?;
        } else if count > 1 {
            // Whether each ray leaves in the direction of the next one.
            let tied: Vec<bool> = (0..count)
                .map(|i| {
                    let here = fan.get(i).map_or(0.0, |&r| angle(r));
                    let next = fan.get((i + 1) % count).map_or(0.0, |&r| angle(r));
                    let gap = wrap(next - here);
                    gap < TIE || gap > TAU - TIE
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
                        order_run(run, &rays, arcs, &sides, point, &frame, tol)?;
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
/// (when the region has one), then the holes. And the imprinted arcs that
/// were dropped because they divide nothing.
#[allow(clippy::type_complexity)]
pub(super) fn build(
    surface: Sheet,
    reversed: bool,
    boundary: &[Vec<Half>],
    interior: &BTreeSet<usize>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<(Vec<Vec<Vec<Half>>>, BTreeSet<usize>), Reason> {
    let mut interior = interior.clone();
    let mut dropped = BTreeSet::new();
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
                return Ok((regions, dropped));
            }
        }
        for slit in slits {
            interior.remove(&slit);
            dropped.insert(slit);
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
    surface: Sheet,
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
    let all_boxes = boxes(&all);
    let flat = matches!(surface.surface, Surface::Plane(_));
    let around = |loop_: usize, about: About| -> Result<bool, Reason> {
        // In a plane a loop does not go around a point outside its box.
        if let About::Point(p) = about
            && flat
            && !all_boxes.around(loop_, p, tol)
        {
            return Ok(false);
        }
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
        let witness = left_of(surface, reversed, &all, &all_boxes, own, 4.0 * tol)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::{EdgeSource, Operand};
    use craft3d_geom::{Curve3, Line, Plane};
    use craft3d_topo::EdgeId;

    fn arc(points: &[Vec3], start: usize, end: usize) -> Arc {
        let (a, b) = (points[start], points[end]);
        Arc {
            curve: Curve3::Line(Line::through(a, b).unwrap()),
            range: (0.0, a.distance(b)),
            start,
            end,
            source: EdgeSource::Kept {
                operand: Operand::A,
                edge: EdgeId(0),
            },
        }
    }

    /// A square with one more arc from a corner: to the opposite corner it
    /// divides the square in two; to the middle it divides nothing, is
    /// dropped, and is reported as dropped (which is how a curve along
    /// which two faces cross, and of which a piece is missing, is caught).
    #[test]
    fn an_arc_that_divides_nothing_is_dropped_and_reported() {
        let points = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 2.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(1.5, 0.5, 0.0),
        ];
        let surface = Surface::Plane(Plane::new(Frame::WORLD));
        let rim: Vec<Half> = (0..4).map(|i| (i, false)).collect();
        let mut arcs: Vec<Arc> = (0..4).map(|i| arc(&points, i, (i + 1) % 4)).collect();
        arcs.push(arc(&points, 0, 2));
        arcs.push(arc(&points, 1, 4));
        let build = |interior: &[usize]| {
            build(
                Sheet::bare(&surface),
                false,
                std::slice::from_ref(&rim),
                &interior.iter().copied().collect(),
                &arcs,
                &points,
                1e-6,
            )
            .unwrap()
        };
        let (regions, dropped) = build(&[4]);
        assert_eq!(regions.len(), 2);
        assert!(dropped.is_empty());
        let (regions, dropped) = build(&[5]);
        assert_eq!(regions, vec![vec![rim.clone()]]);
        assert_eq!(dropped, BTreeSet::from([5]));
        // Both: the diagonal divides, the spur into one half does not.
        let (regions, dropped) = build(&[4, 5]);
        assert_eq!(regions.len(), 2);
        assert_eq!(dropped, BTreeSet::from([5]));
    }
}
