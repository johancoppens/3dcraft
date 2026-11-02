//! A sheet-metal part bent in stages, for the bend sequence check
//! (docs/design/sheetmetal.md › *Bend sequence*): where every flat lies
//! with some bends made, one under way at a stroke angle and the rest
//! still flat; the faces of the part each piece is made of; a slab for
//! a strip not bent yet; the frame the press brake's tools stand in at a
//! bend; and how fast any point of the part can move against the tools
//! per radian of stroke, which turns distances proven at sampled angles
//! into a distance proven over the whole stroke.
//!
//! Every piece of the part at a stage is rigid: a flat with the bends
//! made on it moves as one, a strip not yet bent lies flat on its
//! parent, and the bend under way is the tools' own (its strip and its
//! two flats touch them by design). So the part at a stage is the built
//! part's faces moved piece by piece, never a part built again.

use super::{Part, SheetFace, flat_faces, fold, refuse, world};
use crate::KernelError;
use crate::extrude::extrude;
use crate::profile::{Curve2, Segment, Shape, SketchCurve, detect_profiles, distance};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId};

/// Per flat (0 the base, `i + 1` flange `i`'s), its world transform at
/// the stage where bend `i` is at the signed fold angle `angles[i]` (0
/// flat, the bend's own angle when made), from the pattern's place.
pub fn stage_worlds(
    part: &Part,
    angles: &[f64],
    tolerance: &Tolerance,
) -> Result<Vec<Transform>, KernelError> {
    if angles.len() != part.bends.len() {
        return Err(refuse(
            None,
            format!(
                "a stage gives {} angles for {} bends",
                angles.len(),
                part.bends.len()
            ),
        ));
    }
    let mut bends = part.bends.clone();
    for (b, &a) in bends.iter_mut().zip(angles) {
        if !a.is_finite() || a.abs() > b.angle.abs() * (1.0 + 1e-12) || a * b.angle < 0.0 {
            return Err(refuse(
                Some(b.child.saturating_sub(1)),
                format!(
                    "a stage's angle {a} is not between 0 and the bend's {}",
                    b.angle
                ),
            ));
        }
        // The joint's side (where the axis lies) is the bend's own, also
        // at 0, where its motion is the identity either way.
        b.angle = a;
    }
    let folded = fold(&part.flats, &bends, tolerance)?;
    let plane = Transform::from_frame(&part.plane);
    Ok(folded.transforms.iter().map(|t| t.then(&plane)).collect())
}

/// Per flat, its world transform in the built part (every bend made).
pub fn final_worlds(part: &Part) -> Vec<Transform> {
    part.flats.iter().map(|f| world(&part.plane, f)).collect()
}

/// The faces of the part (indices into `part.solid.body`) made from bend
/// `bend`'s strip: its two skins and the rims along its ends.
pub fn bend_faces(part: &Part, bend: usize) -> Vec<usize> {
    let faces: Vec<FaceId> = part
        .sheet_faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, SheetFace::Bend(x) | SheetFace::BendPiece(x, _) if *x == bend))
        .filter_map(|(f, _)| u32::try_from(f).ok().map(FaceId))
        .collect();
    // The rims along the strip's own edges: those of the bend's faces that
    // no flat has (the tangent lines are shared with flats and end no
    // sheet, so they have no rim).
    let edges: Vec<EdgeId> = faces
        .iter()
        .filter_map(|&face| part.sheet.face(face))
        .flat_map(|f| f.loops.iter())
        .filter_map(|l| part.sheet.loop_(*l))
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .collect();
    part.solid
        .faces
        .iter()
        .enumerate()
        .filter(|(_, made)| match made {
            crate::plate::Made::Front(f) | crate::plate::Made::Back(f) => faces.contains(f),
            crate::plate::Made::Rim(e) => edges.contains(e),
        })
        .map(|(i, _)| i)
        .collect()
}

/// The faces made from flat `flat` (see [`flat_faces`]).
pub fn piece_faces(part: &Part, flat: usize) -> Vec<usize> {
    flat_faces(part, flat)
}

/// Bend `bend`'s strip not yet bent: the slab `T` thick over its strip
/// in the pattern, placed as its parent flat is in the built part (so it
/// moves with the parent's transform from built to stage, as the parent's
/// faces do).
pub fn strip_slab(part: &Part, bend: usize, tolerance: &Tolerance) -> Result<Body, KernelError> {
    let b = part
        .bends
        .get(bend)
        .ok_or_else(|| refuse(Some(bend), "there is no such bend"))?;
    let w = part
        .flats
        .get(b.parent)
        .map(|f| world(&part.plane, f))
        .ok_or_else(|| refuse(Some(bend), "a bend's parent is missing"))?;
    let far = |p: [f64; 2]| {
        [
            p[0] + b.allowance * b.across[0],
            p[1] + b.allowance * b.across[1],
        ]
    };
    // Counter-clockwise in the pattern: the strip lies on the right of
    // `start → end`.
    let corners = [b.end, b.start, far(b.start), far(b.end)];
    let frame = Frame::new(w.point(Vec3::ZERO), w.vector(Vec3::Z), w.vector(Vec3::X))
        .ok_or_else(|| refuse(Some(bend), "a flat's place is not finite"))?;
    prism(&corners, &frame, 0.0, b.thickness, tolerance)
        .map_err(|e| refuse(Some(bend), format!("its strip as a slab: {e}")))
}

/// The closed polygon `points` (in `frame`'s x and y, either way round)
/// extruded along its z from `from` to `to`.
pub fn prism(
    points: &[[f64; 2]],
    frame: &Frame,
    from: f64,
    to: f64,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    if points.len() < 3 {
        return Err(refuse(None, "a polygon needs at least 3 points"));
    }
    let n = points.len();
    let curves: Vec<SketchCurve> = (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some(SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line { start: *a, end: *b },
            })
        })
        .collect();
    let profiles = detect_profiles(&curves, tolerance)?;
    match profiles.as_slice() {
        [one] if one.holes.is_empty() => {}
        _ => {
            return Err(refuse(
                None,
                "the polygon is not one simple closed outline (its sides cross, touch or repeat a point)",
            ));
        }
    }
    Ok(extrude(&profiles, frame, from, to, tolerance)?.body)
}

/// Where the tools stand at a bend under way: on its axis at the middle
/// of the bend, the axis's direction, the unit vector from the axis along
/// the bisector of the two flats towards the inside (where the punch
/// comes from), the inner radius `R` and the thickness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BendFrame {
    pub point: Vec3,
    pub axis: Vec3,
    pub inside: Vec3,
    pub radius: f64,
    pub thickness: f64,
    /// The bend's length along its axis.
    pub reach: f64,
}

impl BendFrame {
    /// The frame of a tool whose origin is `below` from the axis towards
    /// the outside (0 the punch's tip centre, `R + T` the die's point
    /// under the outer skin): `y` the inside, `z` the axis, `x = y × z`.
    pub fn tool(&self, below: f64) -> Option<Frame> {
        let x = self.inside.cross(self.axis);
        Frame::new(self.point - self.inside * below, self.axis, x)
    }
}

/// The frame of bend `bend` at stroke angle `rho` (its size, from 0 to
/// θ), with `worlds` the stage's flat transforms.
pub fn bend_frame(
    part: &Part,
    bend: usize,
    rho: f64,
    worlds: &[Transform],
) -> Result<BendFrame, KernelError> {
    let b = part
        .bends
        .get(bend)
        .ok_or_else(|| refuse(Some(bend), "there is no such bend"))?;
    let wp = worlds
        .get(b.parent)
        .ok_or_else(|| refuse(Some(bend), "a bend's parent is missing"))?;
    let up = b.angle > 0.0;
    let t = b.thickness;
    let zc = if up { t + b.radius } else { -b.radius };
    let mid = [0.5 * (b.start[0] + b.end[0]), 0.5 * (b.start[1] + b.end[1])];
    let along = [b.end[0] - b.start[0], b.end[1] - b.start[1]];
    let point = wp.point(Vec3::new(mid[0], mid[1], zc));
    let axis = wp
        .vector(Vec3::new(along[0], along[1], 0.0))
        .normalized()
        .ok_or_else(|| refuse(Some(bend), "a bend has no length"))?;
    let across = wp.vector(Vec3::new(b.across[0], b.across[1], 0.0));
    let normal = wp.vector(Vec3::Z) * if up { 1.0 } else { -1.0 };
    // The parent's leg runs along −across, the child's along
    // cos ρ across + sin ρ normal: their bisector inwards is
    // (cos ρ − 1) across + sin ρ normal, which is
    // 2 sin(ρ/2) (−sin(ρ/2) across + cos(ρ/2) normal).
    let half = 0.5 * rho;
    let inside = (across * (-half.psin()) + normal * half.pcos())
        .normalized()
        .ok_or_else(|| refuse(Some(bend), "a bend's frame is not finite"))?;
    Ok(BendFrame {
        point,
        axis,
        inside,
        radius: b.radius,
        thickness: t,
        reach: distance(b.start, b.end),
    })
}

/// How fast any point of the part can move against tools standing in a
/// bend's frame, per radian of that bend's stroke, given `reach`: no
/// point of the pieces measured lies further than `reach` from the bend's
/// axis at either end of the stroke.
///
/// Relative to its parent the bend's child turns about the axis by `ρ`
/// after a shift back across the strip by `ρ r` (`r = R + K T`, the arc
/// the neutral fibre takes up: the joint's motion), and the tools' frame
/// turns about the same axis by `ρ/2` (its `y` is the bisector of the two
/// legs). So against the tools a point carried by the parent turns at
/// rate ½ about the axis, at speed `d/2` (`d` its distance from the
/// axis), and one carried by the child at rate ½ the other way plus the
/// shift's `r`. A parent point keeps its distance; a child point's
/// distance is that of `x − ρ r û` (the turn keeps it), a convex function
/// of `ρ`, so at most its larger value at `ρ = 0` and `ρ = θ`, which
/// `reach` bounds. Between two stroke angles `h` apart a distance proven
/// at both ends is proven, less this times `h / 2`, at every angle
/// between.
pub fn stroke_speed(part: &Part, bend: usize, reach: f64) -> Option<f64> {
    let b = part.bends.get(bend)?;
    let speed = 0.5 * reach.max(0.0) + b.neutral();
    speed.is_finite().then_some(speed)
}

/// The distance of `p` from the line through `point` along the unit
/// `axis`, and how far along it `p` lies from `point`.
pub fn from_axis(point: Vec3, axis: Vec3, p: Vec3) -> (f64, f64) {
    let w = p - point;
    let along = w.dot(axis);
    ((w - axis * along).length(), along)
}

/// A piece of the part at a stage, for [`inside_piece`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieceOf {
    /// A flat (0 the base, `i + 1` flange `i`'s).
    Flat(usize),
    /// Bend `i` as built.
    Bend(usize),
    /// Bend `i`'s strip not yet bent, flat on its parent.
    Strip(usize),
}

/// Whether `p` (a point in the built part's world, carried there with the
/// piece) lies strictly inside the material of `piece`: a flat's is its
/// outline (less its holes) `T` thick, a strip's the rectangle over its
/// strip `T` thick, a built bend's the sector of the ring between the
/// radii `R` and `R + T` over its angle, along its length. Exact for any
/// outline (the winding number of its loops); a point on a piece's
/// boundary is on its faces, which the distance sees.
pub fn inside_piece(part: &Part, piece: PieceOf, p: Vec3) -> bool {
    let t = part.thickness;
    let local_of = |flat: usize| {
        part.flats
            .get(flat)
            .and_then(|f| world(&part.plane, f).inverse())
            .map(|w| w.point(p))
    };
    match piece {
        PieceOf::Flat(f) => {
            let (Some(flat), Some(q)) = (part.flats.get(f), local_of(f)) else {
                return false;
            };
            if !(q.z > 0.0 && q.z < t) {
                return false;
            }
            let at = [q.x, q.y];
            let outer = crate::profile::Loop2 {
                segments: flat.outer.clone(),
            };
            outer.winding_about(at).abs() > 0.5
                && flat.holes.iter().all(|h| {
                    crate::profile::Loop2 {
                        segments: h.clone(),
                    }
                    .winding_about(at)
                    .abs()
                        < 0.5
                })
        }
        PieceOf::Bend(i) | PieceOf::Strip(i) => {
            let Some(b) = part.bends.get(i) else {
                return false;
            };
            let Some(q) = local_of(b.parent) else {
                return false;
            };
            let len = distance(b.start, b.end);
            let Some(d) =
                (len > 0.0).then(|| [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len])
            else {
                return false;
            };
            let w = [q.x - b.start[0], q.y - b.start[1]];
            let along = w[0] * d[0] + w[1] * d[1];
            let across = w[0] * b.across[0] + w[1] * b.across[1];
            if !(along > 0.0 && along < len) {
                return false;
            }
            if matches!(piece, PieceOf::Strip(_)) {
                return across > 0.0 && across < b.allowance && q.z > 0.0 && q.z < t;
            }
            // The axis lies `R` beyond the inner skin: above the top skin
            // for a bend up, below the bottom one for a bend down; the
            // strip starts square under (over) it and turns towards
            // `across`.
            let up = b.angle > 0.0;
            let h = if up {
                q.z - (t + b.radius)
            } else {
                q.z + b.radius
            };
            let r = across.phypot(h);
            let phi = across.patan2(if up { -h } else { h });
            r > b.radius && r < b.radius + t && phi > 0.0 && phi < b.theta()
        }
    }
}

/// Points (in the built part's world) whose convex hull holds the
/// material of `piece` exactly as built, with no mesh slack: a flat's the
/// points of its outline's own hull ([`outline_hull`]: the ends of its
/// segments, an arc or circle by the ends of pieces of at most 90 degrees
/// and where each piece's end tangents meet, a spline by its control
/// points) at `z = 0` and `T`, whatever way the outline lies in its
/// axes; a strip's the corners of its rectangle `T` thick; a built bend's
/// the ends of its inner and outer arcs and, for the outer arc taken in
/// two halves of at most 90 degrees, where each half's end tangents
/// meet, at both ends of the bend. Empty for a piece the part does not
/// have.
pub fn piece_points(part: &Part, piece: PieceOf) -> Vec<Vec3> {
    let t = part.thickness;
    let to_world = |flat: usize, local: &[Vec3]| -> Vec<Vec3> {
        part.flats
            .get(flat)
            .map(|f| world(&part.plane, f))
            .map(|w| local.iter().map(|p| w.point(*p)).collect())
            .unwrap_or_default()
    };
    match piece {
        PieceOf::Flat(f) => {
            let Some(flat) = part.flats.get(f) else {
                return Vec::new();
            };
            let hull = outline_hull(&flat.outer);
            if hull.iter().any(|p| !(p[0].is_finite() && p[1].is_finite())) {
                return Vec::new();
            }
            let mut local = Vec::with_capacity(2 * hull.len());
            for p in hull {
                for z in [0.0, t] {
                    local.push(Vec3::new(p[0], p[1], z));
                }
            }
            to_world(f, &local)
        }
        PieceOf::Strip(i) | PieceOf::Bend(i) => {
            let Some(b) = part.bends.get(i) else {
                return Vec::new();
            };
            let lifted = |p: [f64; 2], u: f64, z: f64| {
                Vec3::new(p[0] + u * b.across[0], p[1] + u * b.across[1], z)
            };
            let mut local = Vec::new();
            if matches!(piece, PieceOf::Strip(_)) {
                for p in [b.start, b.end] {
                    for u in [0.0, b.allowance] {
                        for z in [0.0, t] {
                            local.push(lifted(p, u, z));
                        }
                    }
                }
                return to_world(b.parent, &local);
            }
            // In the plane square to the axis: `u` across, `w` up from
            // the axis; the strip starts square under (over) the axis and
            // turns towards `across`.
            let up = b.angle > 0.0;
            let zc = if up { t + b.radius } else { -b.radius };
            let at = |r: f64, phi: f64| {
                let w = if up { -phi.pcos() } else { phi.pcos() };
                (r * phi.psin(), r * w)
            };
            let theta = b.theta();
            let outer = b.radius + t;
            let mut plane = vec![
                at(b.radius, 0.0),
                at(b.radius, theta),
                at(outer, 0.0),
                at(outer, theta),
                at(outer, 0.5 * theta),
            ];
            // Each half arc (at most 90 degrees) lies inside the triangle
            // of its ends and where their tangents meet, `outer /
            // cos(θ/4)` from the axis at the half's middle.
            let apex = outer / (0.25 * theta).pcos();
            if apex.is_finite() {
                plane.push(at(apex, 0.25 * theta));
                plane.push(at(apex, 0.75 * theta));
            }
            for p in [b.start, b.end] {
                for &(u, w) in &plane {
                    local.push(lifted(p, u, zc + w));
                }
            }
            to_world(b.parent, &local)
        }
    }
}

/// The counter-clockwise start angle and sweep (in `(0, 2π]`) of a
/// segment along a circle about `center`: an arc's from its start to its
/// end the way it runs, a circle's whole turn from angle 0.
fn arc_span(segment: &Segment, center: [f64; 2], whole: bool) -> (f64, f64) {
    use std::f64::consts::TAU;
    if whole {
        return (0.0, TAU);
    }
    let angle = |p: [f64; 2]| (p[1] - center[1]).patan2(p[0] - center[0]);
    let (from, to) = if segment.reversed {
        (angle(segment.end), angle(segment.start))
    } else {
        (angle(segment.start), angle(segment.end))
    };
    let sweep = (to - from).rem_euclid(TAU);
    // Ends that are one point make the whole circle.
    (from, if sweep > 0.0 { sweep } else { TAU })
}

/// Points whose convex hull holds the closed outline `outer` (and so the
/// region it bounds, whatever its holes): every segment's ends; an arc or
/// circle cut into pieces of at most 90 degrees, each piece's ends and
/// where their tangents meet (the piece lies in that triangle); a
/// spline's control points (a B-spline lies in their hull).
pub fn outline_hull(outer: &[Segment]) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    for s in outer {
        points.push(s.start);
        points.push(s.end);
        match &s.shape {
            Shape::Line => {}
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let (from, sweep) = arc_span(s, *center, matches!(s.shape, Shape::Circle { .. }));
                let pieces = (sweep / std::f64::consts::FRAC_PI_2).ceil().clamp(1.0, 4.0);
                let step = sweep / pieces;
                let apex = radius / (0.5 * step).pcos();
                let at = |r: f64, a: f64| [center[0] + r * a.pcos(), center[1] + r * a.psin()];
                // `pieces` is 1 to 4.
                for k in 0..(pieces as usize) {
                    let a = from + step * k as f64;
                    points.push(at(*radius, a));
                    points.push(at(apex, a + 0.5 * step));
                }
                points.push(at(*radius, from + sweep));
            }
            Shape::Spline(spline) => points.extend(spline.control.iter().copied()),
        }
    }
    points
}

/// The least of the affine function `height` over the material of
/// `piece` as built: a flat's outline `T` thick (lines at their ends, an
/// arc or circle at its ends or at the one angle where the function's
/// gradient points into the circle, a spline over its control points, a
/// lower bound), a strip's slab at its corners, a built bend's ring
/// sector over its length at an end of its arcs or at that angle on
/// either arc (the function is linear along the radius and the length,
/// so least on the inner or outer arc at an end of the bend). Exact for
/// lines and arcs, so a skin lying along a plane the function is 0 on
/// gives 0, not less. `None` for a piece the part does not have or a
/// value that is not finite.
pub fn piece_lowest(part: &Part, piece: PieceOf, height: impl Fn(Vec3) -> f64) -> Option<f64> {
    let t = part.thickness;
    let flat_of = |piece: PieceOf| match piece {
        PieceOf::Flat(f) => Some(f),
        PieceOf::Bend(i) | PieceOf::Strip(i) => part.bends.get(i).map(|b| b.parent),
    };
    let w = part
        .flats
        .get(flat_of(piece)?)
        .map(|f| world(&part.plane, f))?;
    // The function in the flat's own axes: its gradient (affine, so the
    // same everywhere) only picks the candidate points, each of which is
    // measured by the function itself (an extrapolation from the origin
    // would carry the gradient's rounding over the part's distance from
    // it), at the lower of its two skins for a flat or a strip.
    let f = |p: Vec3| height(w.point(p));
    let o = f(Vec3::ZERO);
    let g = [f(Vec3::X) - o, f(Vec3::Y) - o, f(Vec3::Z) - o];
    let at = |x: f64, y: f64, z: f64| f(Vec3::new(x, y, z));
    let skins = |x: f64, y: f64| at(x, y, 0.0).min(at(x, y, t));
    let mut least = f64::INFINITY;
    match piece {
        PieceOf::Flat(i) => {
            let flat = part.flats.get(i)?;
            for s in &flat.outer {
                for p in [s.start, s.end] {
                    least = least.min(skins(p[0], p[1]));
                }
                match &s.shape {
                    Shape::Line => {}
                    Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                        let (from, sweep) =
                            arc_span(s, *center, matches!(s.shape, Shape::Circle { .. }));
                        // Least along the circle where the point lies
                        // against the gradient.
                        let down = (-g[1]).patan2(-g[0]);
                        if (down - from).rem_euclid(std::f64::consts::TAU) <= sweep {
                            least = least.min(skins(
                                center[0] + radius * down.pcos(),
                                center[1] + radius * down.psin(),
                            ));
                        }
                    }
                    Shape::Spline(spline) => {
                        for p in &spline.control {
                            least = least.min(skins(p[0], p[1]));
                        }
                    }
                }
            }
        }
        PieceOf::Strip(i) => {
            let b = part.bends.get(i)?;
            for p in [b.start, b.end] {
                for u in [0.0, b.allowance] {
                    least = least.min(skins(p[0] + u * b.across[0], p[1] + u * b.across[1]));
                }
            }
        }
        PieceOf::Bend(i) => {
            let b = part.bends.get(i)?;
            // As in `piece_points`: `u` across, `w` up from the axis, the
            // strip starting square under (over) it and turning towards
            // `across`: `(u, w) = (r sin φ, ∓ r cos φ)`.
            let up = b.angle > 0.0;
            let zc = if up { t + b.radius } else { -b.radius };
            let sign = if up { -1.0 } else { 1.0 };
            let gu = g[0] * b.across[0] + g[1] * b.across[1];
            let gw = g[2] * sign;
            // `gu sin φ + gw cos φ` is least at φ = atan2(−gu, −gw).
            let down = (-gu).patan2(-gw);
            let theta = b.theta();
            let mut angles = vec![0.0, theta];
            if down > 0.0 && down < theta {
                angles.push(down);
            }
            for p in [b.start, b.end] {
                for r in [b.radius, b.radius + t] {
                    for &phi in &angles {
                        let (u, wv) = (r * phi.psin(), sign * r * phi.pcos());
                        least =
                            least.min(at(p[0] + u * b.across[0], p[1] + u * b.across[1], zc + wv));
                    }
                }
            }
        }
    }
    least.is_finite().then_some(least)
}

/// The least of the affine function `height` over the material of
/// `piece` as built that lies where the affine function `side` is at
/// least 0: a lower bound, exact for lines and arcs when `side`'s level
/// planes are square to the piece's flat (a flat or a strip cut along a
/// line of its plane) or, for a built bend, square to its axis (cut
/// along its length). `side` is a plane through the material: for a
/// flat or a strip it is taken at the middle of the thickness and the
/// half kept widened by how far the plane leans across the thickness
/// (`|∂side/∂z| T / 2`), so the region kept holds every point of the
/// material where `side ≥ 0`; for a bend the range along its axis is
/// widened by the most `side` takes across its ring sector. The least is
/// then over the outline's pieces in that region (lines at their ends
/// and where they cross the cut, an arc or circle also at the one angle
/// where `height`'s gradient points into it, a spline over its control
/// points: a lower bound), at both skins. `Some(∞)` when no material
/// lies there; `None` for a piece the part does not have or a value
/// that is not a number.
pub fn piece_lowest_within(
    part: &Part,
    piece: PieceOf,
    height: impl Fn(Vec3) -> f64,
    side: impl Fn(Vec3) -> f64,
) -> Option<f64> {
    use std::f64::consts::TAU;
    let t = part.thickness;
    let carrier = match piece {
        PieceOf::Flat(f) => f,
        PieceOf::Bend(i) | PieceOf::Strip(i) => part.bends.get(i)?.parent,
    };
    let w = part.flats.get(carrier).map(|f| world(&part.plane, f))?;
    let f = |p: Vec3| height(w.point(p));
    let s = |p: Vec3| side(w.point(p));
    let o = f(Vec3::ZERO);
    let g = [f(Vec3::X) - o, f(Vec3::Y) - o, f(Vec3::Z) - o];
    let skins = |x: f64, y: f64| f(Vec3::new(x, y, 0.0)).min(f(Vec3::new(x, y, t)));
    if !g.iter().all(|v| v.is_finite()) {
        return None;
    }
    // A flat's or a strip's outline cut by `side` at the middle of the
    // thickness, the half widened by the plane's lean.
    let cut_outline = |outline: &[Segment]| -> Option<f64> {
        let mid = 0.5 * t;
        let lean = 0.5 * (s(Vec3::new(0.0, 0.0, t)) - s(Vec3::ZERO)).abs();
        let v = |p: [f64; 2]| s(Vec3::new(p[0], p[1], mid)) + lean;
        let sv = v([0.0, 0.0]);
        let gs = [v([1.0, 0.0]) - sv, v([0.0, 1.0]) - sv];
        if !(sv.is_finite() && gs[0].is_finite() && gs[1].is_finite() && lean.is_finite()) {
            return None;
        }
        let mut least = f64::INFINITY;
        let mut take = |p: [f64; 2]| least = least.min(skins(p[0], p[1]));
        for seg in outline {
            let (a, b) = (seg.start, seg.end);
            let (va, vb) = (v(a), v(b));
            if va >= 0.0 {
                take(a);
            }
            if vb >= 0.0 {
                take(b);
            }
            match &seg.shape {
                Shape::Line => {
                    if (va < 0.0) != (vb < 0.0) {
                        let k = va / (va - vb);
                        if k.is_finite() {
                            take([a[0] + k * (b[0] - a[0]), a[1] + k * (b[1] - a[1])]);
                        }
                    }
                }
                Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                    let (from, sweep) =
                        arc_span(seg, *center, matches!(seg.shape, Shape::Circle { .. }));
                    let on = |angle: f64| (angle - from).rem_euclid(TAU) <= sweep;
                    let at = |angle: f64| {
                        [
                            center[0] + radius * angle.pcos(),
                            center[1] + radius * angle.psin(),
                        ]
                    };
                    // Where the circle crosses the cut's line.
                    let gl = gs[0].phypot(gs[1]);
                    if gl > 0.0 {
                        let d = v(*center) / gl;
                        let towards = (-gs[1]).patan2(-gs[0]);
                        if d.abs() <= *radius {
                            let spread = (d / radius).clamp(-1.0, 1.0).pacos();
                            for angle in [towards - spread, towards + spread] {
                                if on(angle) {
                                    take(at(angle));
                                }
                            }
                        }
                    }
                    let down = (-g[1]).patan2(-g[0]);
                    if on(down) && v(at(down)) >= 0.0 {
                        take(at(down));
                    }
                }
                Shape::Spline(spline) => {
                    for p in &spline.control {
                        take(*p);
                    }
                }
            }
        }
        Some(least)
    };
    let least = match piece {
        PieceOf::Flat(i) => cut_outline(&part.flats.get(i)?.outer)?,
        PieceOf::Strip(i) => {
            let b = part.bends.get(i)?;
            let far = |p: [f64; 2]| {
                [
                    p[0] + b.allowance * b.across[0],
                    p[1] + b.allowance * b.across[1],
                ]
            };
            let corners = [b.end, b.start, far(b.start), far(b.end)];
            let rect: Vec<Segment> = (0..4)
                .filter_map(|k| {
                    let (a, c) = (corners.get(k)?, corners.get((k + 1) % 4)?);
                    Some(super::line(*a, *c))
                })
                .collect();
            cut_outline(&rect)?
        }
        PieceOf::Bend(i) => {
            let b = part.bends.get(i)?;
            let len = distance(b.start, b.end);
            if !(len > 0.0) {
                return None;
            }
            let d = [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len];
            let up = b.angle > 0.0;
            let zc = if up { t + b.radius } else { -b.radius };
            let sign = if up { -1.0 } else { 1.0 };
            let theta = b.theta();
            // A point of the ring sector `a` along the axis from the
            // bend's start, at radius `r` and angle `phi`.
            let point = |a: f64, r: f64, phi: f64| {
                let (u, wv) = (r * phi.psin(), sign * r * phi.pcos());
                Vec3::new(
                    b.start[0] + a * d[0] + u * b.across[0],
                    b.start[1] + a * d[1] + u * b.across[1],
                    zc + wv,
                )
            };
            // The angles where an affine function of the cross-section is
            // least or most: the arcs' ends and its stationary angle.
            let extreme_angles = |fun: &dyn Fn(Vec3) -> f64| -> Vec<f64> {
                let c0 = fun(point(0.0, 0.0, 0.0));
                let gu = fun(Vec3::new(
                    b.start[0] + b.across[0],
                    b.start[1] + b.across[1],
                    zc,
                )) - c0;
                let gw = (fun(Vec3::new(b.start[0], b.start[1], zc + 1.0)) - c0) * sign;
                let mut angles = vec![0.0, theta];
                for stationary in [(-gu).patan2(-gw), gu.patan2(gw)] {
                    let a = stationary.rem_euclid(TAU);
                    if a > 0.0 && a < theta {
                        angles.push(a);
                    }
                }
                angles
            };
            // The range along the axis that holds every point where
            // `side ≥ 0`: `side` at `a` is at most its value at the
            // sector's point where it is most, plus its slope along `a`.
            let cross: Vec<(f64, f64)> = extreme_angles(&s)
                .into_iter()
                .flat_map(|phi| [(b.radius, phi), (b.radius + t, phi)])
                .collect();
            let most = cross
                .iter()
                .map(|&(r, phi)| s(point(0.0, r, phi)))
                .fold(f64::NEG_INFINITY, f64::max);
            let slope = (s(point(len, 0.0, 0.0)) - s(point(0.0, 0.0, 0.0))) / len;
            if !(most.is_finite() && slope.is_finite()) {
                return None;
            }
            let (mut lo, mut hi) = (0.0_f64, len);
            if slope > 0.0 {
                lo = lo.max(-most / slope);
            } else if slope < 0.0 {
                hi = hi.min(-most / slope);
            } else if most < 0.0 {
                return Some(f64::INFINITY);
            }
            if !(lo <= hi) {
                return Some(f64::INFINITY);
            }
            let mut least = f64::INFINITY;
            for phi in extreme_angles(&f) {
                for r in [b.radius, b.radius + t] {
                    for a in [lo, hi] {
                        least = least.min(f(point(a, r, phi)));
                    }
                }
            }
            least
        }
    };
    (!least.is_nan()).then_some(least)
}

/// The closed polygon `points` (either way round, no three points
/// repeated) cut into triangles by clipping ears: convex pieces whose
/// union is the polygon. `None` when it is not one simple polygon the
/// clipping can cut (a polygon that crosses itself, or a degenerate one).
pub fn triangles(points: &[[f64; 2]]) -> Option<Vec<[[f64; 2]; 3]>> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let area: f64 = (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some(a[0] * b[1] - b[0] * a[1])
        })
        .sum();
    if !(area.abs() > 0.0) {
        return None;
    }
    // Counter-clockwise indices.
    let mut idx: Vec<usize> = (0..n).collect();
    if area < 0.0 {
        idx.reverse();
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0usize;
    while idx.len() > 3 {
        guard += 1;
        if guard > n.saturating_mul(n).saturating_add(8) {
            return None;
        }
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (Some(&ia), Some(&ib), Some(&ic)) =
                (idx.get((k + m - 1) % m), idx.get(k), idx.get((k + 1) % m))
            else {
                return None;
            };
            let (a, b, c) = (*points.get(ia)?, *points.get(ib)?, *points.get(ic)?);
            if !(cross(a, b, c) > 0.0) {
                continue;
            }
            // No other point inside or on the ear.
            let blocked = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let Some(&p) = points.get(j) else {
                    return true;
                };
                cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
            });
            if blocked {
                continue;
            }
            out.push([a, b, c]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    let (Some(&a), Some(&b), Some(&c)) = (idx.first(), idx.get(1), idx.get(2)) else {
        return None;
    };
    let tri = [*points.get(a)?, *points.get(b)?, *points.get(c)?];
    if !(cross(tri[0], tri[1], tri[2]) > 0.0) {
        return None;
    }
    out.push(tri);
    Some(out)
}
