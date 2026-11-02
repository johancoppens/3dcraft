//! A later cut carried back onto the flat pattern
//! (docs/design/sheetmetal.md › *A later cut carried back*): a boolean
//! that drilled the folded part leaves holes in its skins; each hole that
//! goes through the thickness square to the sheet is a hole of the flat
//! pattern, found by laying the skins' new edges flat by the inverse of
//! the fold.
//!
//! **Which faces.** A face of the later body is a skin of the part when its
//! surface is exactly one of the part's skins (a boolean keeps the surfaces
//! of the faces it trims); `Made::Back` is the bottom skin, `Made::Front`
//! the top one.
//!
//! **Laid flat.** A point of a flat's skin goes back by the flat's inverse
//! motion (its `z` is the skin's level, 0 or `T`); a point of a bend's skin
//! by the inverse wrap: `u` along the axis, the angle `φ` about it from the
//! parent's tangent line, `s = φ (R + K T)` across the strip.
//!
//! **New edges.** An edge of a skin that does not lie on the pattern's own
//! curves (its outline, holes, cuts and tangent lines; three points within
//! twice the tolerance) is the later feature's. Laid flat, they are chained
//! end to end into loops on each skin; a loop of the bottom skin whose
//! points all lie within four tolerances of a loop of the top skin, and the
//! other way round, is a hole through the thickness square to the sheet,
//! carried. One that meets a loop of the other skin without matching it is
//! a cut that is not square to the sheet (refused); a loop on one skin only
//! (a pocket, an emboss, a part joined on) and a chain that does not close
//! (a cut that reaches the edge and changes the outline) are not carried,
//! and said so.

use super::{P2, Part, Segment, Shape, SheetFace, cross, distance, dot, outline, sub, unit};
use crate::KernelError;
use crate::plate::Made;
use crate::profile::Loop2;
use crate::spline2::Spline2;
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Body;

/// What a later body's holes are in the flat pattern.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LaterCuts {
    /// The holes through the thickness, square to the sheet, as loops of
    /// the pattern (clockwise).
    pub holes: Vec<Vec<Segment>>,
    /// What was not carried, and why.
    pub not_carried: Vec<String>,
}

fn refuse(reason: impl Into<String>) -> KernelError {
    KernelError::SheetMetal {
        flange: None,
        reason: reason.into(),
    }
}

/// The pattern point a point `p` (in the world) of the skin at `level`
/// (0 the bottom, 1 the top) of the sheet face `role` lies flat at, with
/// how far `p` is off that skin.
fn flat_point(part: &Part, role: SheetFace, level: usize, p: Vec3) -> Option<(P2, f64)> {
    let t = part.thickness;
    let (x, y, z) = part.plane.local(p);
    let local = Vec3::new(x, y, z);
    match role {
        SheetFace::Flat(i) | SheetFace::FlatPiece(i, _) => {
            let q = part.flats.get(i)?.transform.inverse()?.point(local);
            let at = if level == 0 { 0.0 } else { t };
            Some(([q.x, q.y], (q.z - at).abs()))
        }
        SheetFace::Bend(n) | SheetFace::BendPiece(n, _) => {
            let b = part.bends.get(n)?;
            let q = part.flats.get(b.parent)?.transform.inverse()?.point(local);
            let d = unit(sub(b.end, b.start))?;
            let u = dot(sub([q.x, q.y], b.start), d);
            let zc = if b.angle > 0.0 {
                t + b.radius
            } else {
                -b.radius
            };
            let foot = [b.start[0] + u * d[0], b.start[1] + u * d[1]];
            let v = [dot(sub([q.x, q.y], foot), b.across), q.z - zc];
            let r0 = [0.0, -zc];
            // Signed: positive towards the flange (a bend up turns with
            // the axis above, a bend down below).
            let phi = cross(r0, v).patan2(dot(r0, v)) * zc.signum();
            // The bottom skin is the outside of a bend up.
            let bottom = zc.abs();
            let radius = match (level, b.angle > 0.0) {
                (0, _) => bottom,
                (_, true) => bottom - t,
                (_, false) => bottom + t,
            };
            let s = phi * b.neutral();
            Some((
                [foot[0] + s * b.across[0], foot[1] + s * b.across[1]],
                (v[0].phypot(v[1]) - radius).abs(),
            ))
        }
    }
}

fn line(start: P2, end: P2) -> Segment {
    Segment {
        source: 0,
        piece: 0,
        reversed: false,
        start,
        end,
        shape: Shape::Line,
    }
}

/// The simplest pattern segment through `points` (a line, an arc or
/// circle, else a cubic spline through them) that holds them and the
/// `middles` (the points half way between) within a quarter of `tol`.
fn segment_through(points: &[P2], middles: &[P2], tol: f64) -> Option<Segment> {
    let limit = 0.25 * tol;
    let (&first, &last) = (points.first()?, points.last()?);
    let all = || points.iter().chain(middles);
    let closed = distance(first, last) <= tol;
    if !closed {
        let chord = line(first, last);
        if all().all(|p| chord.distance_to(*p, tol) <= limit) {
            return Some(chord);
        }
    }
    // The circle through three of them.
    let (a, b, c) = if closed {
        (
            first,
            *points.get(points.len() / 3)?,
            *points.get(2 * points.len() / 3)?,
        )
    } else {
        (first, *points.get(points.len() / 2)?, last)
    };
    let (ab, ac) = (sub(b, a), sub(c, a));
    let den = 2.0 * cross(ab, ac);
    if den.abs() > 0.0 {
        let (lb, lc) = (dot(ab, ab), dot(ac, ac));
        let center = [
            a[0] + (ac[1] * lb - ab[1] * lc) / den,
            a[1] + (ab[0] * lc - ac[0] * lb) / den,
        ];
        let radius = distance(center, a);
        let fits =
            radius.is_finite() && all().all(|p| (distance(*p, center) - radius).abs() <= limit);
        if fits {
            let ccw = cross(sub(b, a), sub(c, b)) > 0.0;
            let mut seg = Segment {
                source: 0,
                piece: 0,
                reversed: !ccw,
                start: first,
                end: if closed { first } else { last },
                shape: if closed {
                    Shape::Circle { center, radius }
                } else {
                    Shape::Arc { center, radius }
                },
            };
            if closed {
                // A circle starts at its angle 0.
                let at0 = [center[0] + radius, center[1]];
                seg.start = at0;
                seg.end = at0;
            }
            return Some(seg);
        }
    }
    // A cubic spline through the points, at even parameters.
    let n = points.len();
    if n < 4 {
        return None;
    }
    let params: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p[0], p[1]]).collect();
    let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, 3, &values).ok()?;
    let control: Vec<[f64; 2]> = ctrl
        .iter()
        .map(|c| {
            [
                c.first().copied().unwrap_or(f64::NAN),
                c.get(1).copied().unwrap_or(f64::NAN),
            ]
        })
        .collect();
    let curve = craft3d_geom::NurbsCurve::polynomial(
        3,
        knots.clone(),
        control.iter().map(|c| Vec3::new(c[0], c[1], 0.0)).collect(),
    )
    .ok()?;
    let held = middles.iter().enumerate().all(|(i, m)| {
        let t = (i as f64 + 0.5) / (n - 1) as f64;
        let e = curve.eval(t);
        distance([e.x, e.y], *m) <= limit
    });
    if !held {
        return None;
    }
    let (Some(&s0), Some(&s1)) = (control.first(), control.last()) else {
        return None;
    };
    Some(Segment {
        source: 0,
        piece: 0,
        reversed: false,
        start: s0,
        end: s1,
        shape: Shape::Spline(Spline2 {
            degree: 3,
            knots,
            control,
        }),
    })
}

/// The edges of `body`'s skins of the part that are not on the pattern's
/// curves, laid flat: per skin (0 bottom, 1 top) the pattern segments.
fn new_edges(
    part: &Part,
    body: &Body,
    tolerance: &Tolerance,
) -> Result<[Vec<Segment>; 2], KernelError> {
    let tol = tolerance.linear;
    let mut own: Vec<Segment> = outline(part, tolerance).into_iter().flatten().collect();
    for b in &part.bends {
        let (a0, a1) = b.far_line();
        own.push(line(b.start, b.end));
        own.push(line(a0, a1));
    }
    let on_own = |p: P2| own.iter().any(|s| s.distance_to(p, tol) <= 2.0 * tol);
    let mut out: [Vec<Segment>; 2] = [Vec::new(), Vec::new()];
    for face in &body.faces {
        // The skin it is, by its surface.
        let skins: Vec<(usize, craft3d_topo::FaceId)> = part
            .solid
            .body
            .faces
            .iter()
            .zip(&part.solid.faces)
            .filter(|(f, _)| f.surface == face.surface)
            .filter_map(|(_, made)| match made {
                Made::Back(s) => Some((0usize, *s)),
                Made::Front(s) => Some((1usize, *s)),
                // A sharp bend's outer skin is no skin a later cut is
                // laid flat from (owed): a cut across it is not carried.
                Made::Rim(_) | Made::Joined(_) => None,
            })
            .collect();
        // Several skins may share a surface (the faces a hole across a
        // bend leaves a flat or a bend in): they lie flat the same way
        // when they are of one level and one flat or bend.
        let role_of = |s: &craft3d_topo::FaceId| {
            usize::try_from(s.0)
                .ok()
                .and_then(|i| part.sheet_faces.get(i))
                .copied()
        };
        let cell = |r: SheetFace| match r {
            SheetFace::Flat(i) | SheetFace::FlatPiece(i, _) => (0, i),
            SheetFace::Bend(i) | SheetFace::BendPiece(i, _) => (1, i),
        };
        let Some(&(level, first)) = skins.first() else {
            continue;
        };
        // A skin that cannot be laid flat one way is refused, never left
        // out: a later cut through it would be missing from the pattern.
        let Some(role) = role_of(&first) else {
            return Err(refuse(
                "a face of the part's skin is not a flat or a bend of its pattern, so the later cuts through it cannot be laid flat".to_string(),
            ));
        };
        let agree = skins
            .iter()
            .all(|(l, s)| *l == level && role_of(s).map(cell) == Some(cell(role)));
        if !agree {
            return Err(refuse(
                "a face of the part's skin lies on the surface of skins of different flats, bends or sides, so the later cuts through it cannot be laid flat one way".to_string(),
            ));
        }
        for l in &face.loops {
            let Some(lp) = body.loop_(*l) else { continue };
            for c in &lp.coedges {
                let Some(edge) = body.edge(c.edge) else {
                    continue;
                };
                let (t0, t1) = edge.range;
                let mut count = 16usize;
                let made = loop {
                    let at = |i: usize, n: usize| t0 + (t1 - t0) * i as f64 / n as f64;
                    let mut points = Vec::with_capacity(count + 1);
                    let mut middles = Vec::with_capacity(count);
                    for i in 0..=count {
                        let (p, off) = flat_point(part, role, level, edge.curve.eval(at(i, count)))
                            .ok_or_else(|| refuse("a skin of the part cannot be laid flat"))?;
                        if !(off <= 4.0 * tol) {
                            return Err(refuse(format!(
                                "a face on a skin of the part has an edge {off} off that skin; the later feature's body is not the part with cuts in it"
                            )));
                        }
                        points.push(p);
                        if i < count {
                            let (m, _) = flat_point(
                                part,
                                role,
                                level,
                                edge.curve.eval(at(2 * i + 1, 2 * count)),
                            )
                            .ok_or_else(|| refuse("a skin of the part cannot be laid flat"))?;
                            middles.push(m);
                        }
                    }
                    let is_own = [0.25, 0.5, 0.75].iter().all(|f| {
                        let i = ((count as f64) * f).round() as usize;
                        points.get(i).is_some_and(|p| on_own(*p))
                    });
                    if is_own {
                        break None;
                    }
                    if let Some(seg) = segment_through(&points, &middles, tol) {
                        break Some(seg);
                    }
                    if count >= 2048 {
                        return Err(refuse(
                            "an edge of a later cut could not be laid flat within a quarter of the tolerance",
                        ));
                    }
                    count *= 2;
                };
                if let (Some(seg), Some(list)) = (made, out.get_mut(level)) {
                    list.push(seg);
                }
            }
        }
    }
    Ok(out)
}

/// Chains segments end to end (ends within `reach`) into loops (clockwise)
/// and the chains that do not close.
fn chain(mut segs: Vec<Segment>, reach: f64) -> (Vec<Vec<Segment>>, usize) {
    let mut loops = Vec::new();
    let mut open = 0;
    while let Some(first) = segs.pop() {
        if distance(first.start, first.end) <= reach && !matches!(first.shape, Shape::Line) {
            loops.push(clockwise(vec![first]));
            continue;
        }
        let start = first.start;
        let mut ring = vec![first];
        let mut closed = false;
        for _ in 0..=segs.len() {
            let Some(end) = ring.last().map(|s| s.end) else {
                break;
            };
            if distance(end, start) <= reach && ring.len() > 1 {
                closed = true;
                break;
            }
            let next = segs
                .iter()
                .position(|s| distance(s.start, end) <= reach || distance(s.end, end) <= reach);
            let Some(j) = next else { break };
            let s = segs.swap_remove(j);
            let s = if distance(s.start, end) <= reach {
                s
            } else {
                opposite(&s)
            };
            // Each piece starts where the one before it ends.
            ring.push(Segment { start: end, ..s });
        }
        if closed {
            if let Some(last) = ring.last_mut() {
                last.end = start;
            }
            loops.push(clockwise(ring));
        } else {
            open += 1;
        }
    }
    (loops, open)
}

/// The loop run clockwise, as a hole's is.
fn clockwise(ring: Vec<Segment>) -> Vec<Segment> {
    let area = Loop2 {
        segments: ring.clone(),
    }
    .signed_area();
    if area > 0.0 {
        Loop2 { segments: ring }.flipped().segments
    } else {
        ring
    }
}

fn opposite(seg: &Segment) -> Segment {
    Segment {
        source: seg.source,
        piece: seg.piece,
        reversed: !seg.reversed,
        start: seg.end,
        end: seg.start,
        shape: seg.shape.clone(),
    }
}

/// Points along a loop: each segment's ends and middle.
fn samples(ring: &[Segment]) -> Vec<P2> {
    ring.iter()
        .flat_map(|s| [s.start, s.point_at(0.25), s.point_at(0.5), s.point_at(0.75)])
        .collect()
}

fn bbox(points: &[P2]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

/// The holes `body` (a later feature's body made from the part) has in the
/// part's skins, laid flat (see the module documentation).
///
/// **Errors.** A hole on both skins that does not go through square to the
/// sheet (its two skins' loops differ by more than four tolerances), and a
/// skin that cannot be laid flat.
pub fn later_cuts(
    part: &Part,
    body: &Body,
    tolerance: &Tolerance,
) -> Result<LaterCuts, KernelError> {
    let tol = tolerance.linear;
    let [bottom, top] = new_edges(part, body, tolerance)?;
    let reach = 8.0 * tol;
    let (bottom, open_b) = chain(bottom, reach);
    let (top, open_t) = chain(top, reach);
    let mut found = LaterCuts::default();
    if open_b + open_t > 0 {
        found.not_carried.push(format!(
            "{} cut(s) reach the edge of the part or of a hole and change its outline, which is not carried onto the flat pattern (owed); draw them in the part's sketch or a cutouts sketch",
            open_b.max(open_t)
        ));
    }
    let matched = |a: &[Segment], b: &[Segment]| {
        samples(a)
            .iter()
            .all(|p| b.iter().any(|s| s.distance_to(*p, tol) <= 4.0 * tol))
    };
    let mut used_top = vec![false; top.len()];
    for ring in bottom {
        let pts = samples(&ring);
        let bb = bbox(&pts);
        let mut paired = false;
        let mut met = false;
        for (j, other) in top.iter().enumerate() {
            let ob = bbox(&samples(other));
            let meet = !(ob[0] > bb[2] || ob[2] < bb[0] || ob[1] > bb[3] || ob[3] < bb[1]);
            if !meet {
                continue;
            }
            met = true;
            if matched(&ring, other) && matched(other, &ring) {
                if let Some(u) = used_top.get_mut(j) {
                    *u = true;
                }
                paired = true;
                break;
            }
        }
        if met && !paired {
            return Err(refuse(format!(
                "a later cut near ({}, {}) in the flat pattern does not go through the sheet square to it: its outline on the two skins differs, so it has no one outline in the flat (a bevelled or tilted cut, or one across a bend whose walls do not follow the bend's radial lines); cut it square to the sheet, or draw it in the part's sketch or a cutouts sketch",
                bb[0], bb[1]
            )));
        }
        if paired {
            found.holes.push(ring);
        } else {
            found.not_carried.push(format!(
                "a cut near ({}, {}) in the flat pattern is on the bottom skin only (a pocket, or a part joined on), not through the thickness: not carried",
                bb[0], bb[1]
            ));
        }
    }
    for (ring, used) in top.iter().zip(used_top) {
        if !used {
            let bb = bbox(&samples(ring));
            found.not_carried.push(format!(
                "a cut near ({}, {}) in the flat pattern is on the top skin only (a pocket, or a part joined on), not through the thickness: not carried",
                bb[0], bb[1]
            ));
        }
    }
    Ok(found)
}
