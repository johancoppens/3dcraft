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
use crate::profile::{Curve2, SketchCurve, detect_profiles, distance};
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
    let inside = (across * (-half.sin()) + normal * half.cos())
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
            let r = across.hypot(h);
            let phi = across.atan2(if up { -h } else { h });
            r > b.radius && r < b.radius + t && phi > 0.0 && phi < b.theta()
        }
    }
}
