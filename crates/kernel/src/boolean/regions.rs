//! The pieces a face falls into when curves are imprinted on it.
//!
//! The boundary pieces of the face (each used one way) and the imprinted
//! pieces (each used both ways) form a planar map on the face's surface.
//! Around every vertex the pieces that leave it are sorted by direction in
//! the tangent plane; a region's boundary is followed by always taking, at
//! the end of a piece, the next piece clockwise from the way back, which
//! keeps the region on the left. Every use of a piece ends up in exactly
//! one such loop.
//!
//! Loops are then grouped into regions. On a plane a counter-clockwise
//! loop is the outside of a region and a clockwise one a hole in the
//! smallest region around it. On a cylinder a loop may also go around the
//! axis: such loops are sorted along the axis and paired, lower with upper,
//! into bands.

use super::outline::{
    Seg, Side, clear_angle, side_of, signed_area, surface_normal, top_crossing, turns,
};
use super::{Arc, NEAR, Reason};
use craft3d_geom::Surface;
use craft3d_math::{Frame, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// The use of an arc: the arc's index and whether it is run backwards.
pub(super) type Half = (usize, bool);

/// Directions closer together than this (radians) at a vertex are tangent
/// curves: which comes first cannot be told reliably.
const ANGLE_GUARD: f64 = 1e-6;

const TANGENT: Reason = "two curves leave a point in the same direction (tangent contact)";
const TOUCH: Reason =
    "the bodies touch along an edge without overlapping there (the result would not be a manifold)";
const LOST: Reason = "an imprinted face could not be divided into regions; this is a bug";
const UNBOUNDED: Reason = "a piece of a cylindrical face is not closed off by its boundary";

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
            closed: arc.start == arc.end,
            reversed,
        });
    }
    Ok(out)
}

/// The vertex a use of an arc leaves from, and its direction there.
fn leaving(half: Half, arcs: &[Arc]) -> Result<(usize, Vec3), Reason> {
    let arc = arcs.get(half.0).ok_or(LOST)?;
    let (vertex, direction) = if half.1 {
        (arc.end, -arc.curve.derivatives(arc.range.1).1)
    } else {
        (arc.start, arc.curve.derivatives(arc.range.0).1)
    };
    Ok((vertex, direction.normalized().ok_or(LOST)?))
}

/// The regions of a face: each a list of loops, the outer boundary (or the
/// two rims of a band) first, then the holes.
pub(super) fn build(
    surface: &Surface,
    reversed: bool,
    boundary: &[Vec<Half>],
    interior: &BTreeSet<usize>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<Vec<Vec<Vec<Half>>>, Reason> {
    let mut halves: Vec<Half> = boundary.iter().flatten().copied().collect();
    for &arc in interior {
        halves.push((arc, false));
        halves.push((arc, true));
    }

    // Tangent-plane axes per vertex, counter-clockwise seen from outside.
    let mut frames: BTreeMap<usize, Frame> = BTreeMap::new();
    let mut angle_at = |vertex: usize, direction: Vec3| -> Result<f64, Reason> {
        let frame = match frames.get(&vertex) {
            Some(frame) => *frame,
            None => {
                let point = *points.get(vertex).ok_or(LOST)?;
                let normal = surface_normal(surface, point).ok_or(LOST)?;
                let outward = if reversed { -normal } else { normal };
                let frame = Frame::from_z(point, outward).ok_or(LOST)?;
                frames.insert(vertex, frame);
                frame
            }
        };
        Ok(direction.dot(frame.y()).atan2(direction.dot(frame.x())))
    };

    // Per vertex, the uses that leave it, by angle.
    let mut around: BTreeMap<usize, Vec<(f64, usize)>> = BTreeMap::new();
    for (index, &half) in halves.iter().enumerate() {
        let (vertex, direction) = leaving(half, arcs)?;
        let angle = angle_at(vertex, direction)?;
        around.entry(vertex).or_default().push((angle, index));
    }
    for fan in around.values_mut() {
        fan.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let next = fan.iter().cycle().skip(1);
        if fan.len() > 1 {
            for (here, there) in fan.iter().zip(next) {
                let gap = wrap(there.0 - here.0);
                if gap < ANGLE_GUARD || gap > TAU - ANGLE_GUARD {
                    return Err(TANGENT);
                }
            }
        }
    }

    // The use that follows each use around its region.
    let mut following = Vec::with_capacity(halves.len());
    for &half in &halves {
        let twin = (half.0, !half.1);
        let (vertex, back) = leaving(twin, arcs)?;
        let back = angle_at(vertex, back)?;
        let fan = around.get(&vertex).ok_or(TOUCH)?;
        let mut best: Option<(f64, usize)> = None;
        for &(angle, index) in fan {
            if halves.get(index) == Some(&twin) {
                continue;
            }
            let clockwise = wrap(back - angle);
            if clockwise < ANGLE_GUARD || clockwise > TAU - ANGLE_GUARD {
                return Err(TANGENT);
            }
            if best.is_none_or(|(least, _)| clockwise < least) {
                best = Some((clockwise, index));
            }
        }
        // Nothing but the way back: an imprinted curve that ends in the
        // middle of the face.
        following.push(best.ok_or(TOUCH)?.1);
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
        // An imprinted curve with the same region on both sides.
        let mut used = BTreeSet::new();
        for half in &loop_ {
            if !used.insert(half.0) && interior.contains(&half.0) {
                return Err(TOUCH);
            }
        }
        loops.push(loop_);
    }

    group(surface, reversed, loops, arcs, points, tol)
}

struct Measured {
    halves: Vec<Half>,
    segs: Vec<Seg>,
    /// Times around the axis, in the sense in which a rim that has its face
    /// above it counts +1.
    around: i32,
    area: f64,
}

fn group(
    surface: &Surface,
    reversed: bool,
    loops: Vec<Vec<Half>>,
    arcs: &[Arc],
    points: &[Vec3],
    tol: f64,
) -> Result<Vec<Vec<Vec<Half>>>, Reason> {
    let mut measured = Vec::with_capacity(loops.len());
    for halves in loops {
        let segs = segs(&halves, arcs, points)?;
        let around = turns(surface, &segs)? * if reversed { -1 } else { 1 };
        let area = if around == 0 {
            signed_area(surface, reversed, &segs)?
        } else {
            0.0
        };
        if around == 0 && area.abs() <= tol * tol {
            return Err("a piece of a face with no area (at the edge of tolerance)");
        }
        measured.push(Measured {
            halves,
            segs,
            around,
            area,
        });
    }

    // A region: its outer loops (one, or the two rims of a band), its area
    // when it has one outer loop, and its holes.
    let mut regions: Vec<(Vec<usize>, Option<f64>, Vec<usize>)> = Vec::new();
    for (index, m) in measured.iter().enumerate() {
        if m.around == 0 && m.area > 0.0 {
            regions.push((vec![index], Some(m.area), Vec::new()));
        }
    }
    // Rims, from the bottom of the cylinder up; where two share their
    // height (the two sides of one imprinted circle) the upper rim of the
    // band below comes first.
    let mut rims: Vec<(f64, i32, usize)> = Vec::new();
    let winding: Vec<&[Seg]> = measured
        .iter()
        .filter(|m| m.around != 0)
        .map(|m| m.segs.as_slice())
        .collect();
    let ruling = clear_angle(surface, &winding);
    for (index, m) in measured.iter().enumerate() {
        if m.around != 0 {
            let height = top_crossing(surface, &m.segs, ruling)?.ok_or(UNBOUNDED)?;
            rims.push((height, m.around, index));
        }
    }
    rims.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut pending: Option<usize> = None;
    for (_, around, index) in rims {
        match (pending, around) {
            (None, 1) => pending = Some(index),
            (Some(lower), -1) => {
                regions.push((vec![lower, index], None, Vec::new()));
                pending = None;
            }
            _ => return Err(UNBOUNDED),
        }
    }
    if pending.is_some() {
        return Err(UNBOUNDED);
    }

    for (index, m) in measured.iter().enumerate() {
        if !(m.around == 0 && m.area < 0.0) {
            continue;
        }
        let probe = m
            .segs
            .first()
            .map(|s| s.curve.eval(s.middle()))
            .ok_or(LOST)?;
        // The smallest region around the hole; a band when no other is.
        let mut home: Option<(usize, f64)> = None;
        let own: BTreeSet<usize> = m.halves.iter().map(|h| h.0).collect();
        for (at, (outer, area, _)) in regions.iter().enumerate() {
            // A loop that shares an arc with the hole has it from the
            // other side: it bounds what is in the hole, or beside it.
            let beside = outer
                .iter()
                .filter_map(|&i| measured.get(i))
                .any(|o| o.halves.iter().any(|h| own.contains(&h.0)));
            if beside {
                continue;
            }
            let outer_loops: Vec<Vec<Seg>> = outer
                .iter()
                .filter_map(|&i| measured.get(i).map(|m| m.segs.clone()))
                .collect();
            match side_of(surface, &outer_loops, probe, tol)? {
                Side::Out => continue,
                Side::On => return Err(NEAR),
                Side::In => {}
            }
            let size = area.unwrap_or(f64::INFINITY);
            if home.is_none_or(|(_, least)| size < least) {
                home = Some((at, size));
            }
        }
        let (at, _) = home.ok_or(LOST)?;
        regions.get_mut(at).ok_or(LOST)?.2.push(index);
    }

    regions.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::with_capacity(regions.len());
    for (outer, _, holes) in regions {
        let mut region = Vec::with_capacity(outer.len() + holes.len());
        for index in outer.into_iter().chain(holes) {
            region.push(measured.get(index).ok_or(LOST)?.halves.clone());
        }
        out.push(region);
    }
    Ok(out)
}
