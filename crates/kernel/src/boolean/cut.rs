//! A solid cut by a sheet: which regions of the solid's faces lie in
//! front of the sheet and which behind it.
//!
//! A sheet has no inside, so the regions of the solid are not classified
//! by rays against it. They are told apart by where they meet the sheet's
//! piece inside the solid (the *cap*, the sheet's regions that lie inside
//! the solid, classified by rays as any region is): along an arc of the
//! cap's boundary the region of a face of the solid on it is in front of
//! the sheet when the direction from the arc into the region points the
//! way the sheet's normal points, and behind when it points against it.
//! Regions that share an arc that is no cut are on one side. The sheet
//! has to reach across the solid: every arc of the cap's boundary lies on
//! the solid's boundary, every region is reached from a cut, and no
//! region is on both sides; otherwise the cut is refused with the reason.

use super::classify::{FaceRegions, Kept, face_regions};
use super::outline::outward_normal;
use super::regions::Half;
use super::section::Solid;
use super::sew::{root, unite};
use super::{Arcs, BooleanOp, FaceSource, LOST, Reason, Stop, Vertices, face_id};
use craft3d_math::Tolerance;
use std::collections::{BTreeMap, BTreeSet};

/// The sheet ends inside the solid.
pub(super) const SHEET_ENDS_INSIDE: Reason = "the sheet ends inside the solid: to cut a solid a sheet must reach across it, its boundary outside the solid or on its faces";
/// No piece of the sheet lies inside the solid.
pub(super) const NOT_CUT: Reason = "the sheet does not pass through the solid: no piece of it lies inside the solid, so it cuts nothing";
/// A region is reached from both sides of the sheet.
pub(super) const BOTH_SIDES: Reason =
    "the sheet does not divide the solid: a part of it lies on both sides of the sheet";
/// A shell of the solid that the sheet does not cross.
pub(super) const NOT_CROSSED: Reason = "a part of the solid that the sheet does not cross: which side of an open sheet it lies on is not told (cut a solid of one part, or cut its parts one by one)";
/// The sheet meets a face of the solid along a curve where they are
/// tangent.
pub(super) const TANGENT_CUT: Reason = "the sheet meets a face of the solid tangentially along a curve: which side is which cannot be told there";

/// Below this the direction into a region and the sheet's normal are
/// taken to be square to each other (radians, as a sine).
const SQUARE: f64 = 1e-6;

/// The regions of `solid` (operand A) on the side of the sheet that `op`
/// keeps (subtract: in front, where the sheet's normal points; intersect:
/// behind), added to `kept`. `cap` are the sheet's regions inside the
/// solid, as kept (turned over for subtract); `sheet` the prepared sheet
/// they are regions of.
#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    solid: &Solid,
    sheet: &Solid,
    cap: &[Kept],
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    arcs: &Arcs,
    vertices: &Vertices,
    op: BooleanOp,
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    if cap.is_empty() {
        return Err(Stop::of(NOT_CUT));
    }
    // The cap's boundary is where it meets the solid's boundary: every
    // arc of it that only one piece of the cap uses lies along an edge of
    // the solid or across one of its faces. (Checked before the solid's
    // faces are divided: where the sheet ends inside, a curve it crosses a
    // face in ends inside that face and divides nothing.)
    let mut on_solid: BTreeSet<usize> = pieces.iter().flatten().map(|&(arc, _)| arc).collect();
    on_solid.extend(interior.values().flatten().copied());
    let mut by_cap: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (c, piece) in cap.iter().enumerate() {
        for &(arc, _) in piece.loops.iter().flatten() {
            by_cap.entry(arc).or_default().push(c);
        }
    }
    for (arc, uses) in &by_cap {
        if uses.len() == 1 && !on_solid.contains(arc) {
            let named: Vec<_> = uses
                .iter()
                .filter_map(|&c| cap.get(c))
                .map(|k| (k.source.operand, k.source.face))
                .collect();
            return Err(Stop::of(SHEET_ENDS_INSIDE).at(&named));
        }
    }
    let sheet_box = sheet.bbox();
    let none = BTreeMap::new();
    // Every region of the solid's faces: its face and its loops.
    let mut regions: Vec<(usize, Vec<Vec<Half>>)> = Vec::new();
    for face in 0..solid.faces() {
        // A region on the sheet's surface is told its side as any other:
        // where a piece of the sheet lies on it, that piece was refused.
        let coincident = coincident.get(&face).map_or(&[][..], Vec::as_slice);
        // No curve is held to dividing a face here: every curve the sheet
        // crosses a face of the solid in is a cut, and one that divides
        // nothing (a ring cut once) is told below as a sheet that does not
        // divide the solid.
        let FaceRegions { regions: found, .. } = face_regions(
            solid, face, pieces, interior, &none, coincident, arcs, vertices, &sheet_box, tol,
        )?;
        regions.extend(found.into_iter().map(|loops| (face, loops)));
    }
    // Per arc, its uses by regions of the solid.
    let mut by_solid: BTreeMap<usize, Vec<(usize, bool)>> = BTreeMap::new();
    for (r, (_, loops)) in regions.iter().enumerate() {
        for &(arc, reversed) in loops.iter().flatten() {
            by_solid.entry(arc).or_default().push((r, reversed));
        }
    }
    // An arc of the cap's boundary that no region of the solid ends at
    // runs across a face without dividing it: the material on its two
    // sides is one.
    for (arc, uses) in &by_cap {
        if uses.len() == 1 && !by_solid.contains_key(arc) {
            let named: Vec<_> = uses
                .iter()
                .filter_map(|&c| cap.get(c))
                .map(|k| (k.source.operand, k.source.face))
                .collect();
            return Err(Stop::of(BOTH_SIDES).at(&named));
        }
    }
    // Regions joined by an arc that is no cut are on one side.
    let mut parents: Vec<usize> = (0..regions.len()).collect();
    for (arc, uses) in &by_solid {
        if by_cap.contains_key(arc) {
            continue;
        }
        if let Some(&(first, _)) = uses.first() {
            for &(other, _) in uses.iter().skip(1) {
                unite(&mut parents, first, other);
            }
        }
    }
    // The side of each group, from every cut that bounds a region of it:
    // +1 in front of the sheet, −1 behind.
    let mut side: BTreeMap<usize, i8> = BTreeMap::new();
    for (arc_index, caps) in &by_cap {
        let Some(uses) = by_solid.get(arc_index) else {
            continue;
        };
        let arc = arcs.arcs.get(*arc_index).ok_or(Stop::of(LOST))?;
        let middle = 0.5 * (arc.range.0 + arc.range.1);
        let (point, tangent) = arc.curve.derivatives(middle);
        // The sheet's own normal there (not the cap's, which subtract has
        // turned over), from every face of the sheet along the arc.
        let mut normals = Vec::with_capacity(caps.len());
        for &c in caps {
            let piece = cap.get(c).ok_or(Stop::of(LOST))?;
            let face = piece.source.face.0 as usize;
            let outline = sheet.outline(face)?;
            let normal = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&[sheet.name(face)]))?;
            normals.push((normal, sheet.name(face)));
        }
        for &(r, reversed) in uses {
            let (face, _) = regions.get(r).ok_or(Stop::of(LOST))?;
            let named = [solid.name(*face)];
            let outline = solid.outline(*face)?;
            let outward = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&named))?;
            let travel = if reversed { -tangent } else { tangent };
            // Seen from outside the face lies to the left of its loops.
            let Some(into) = outward.cross(travel).normalized() else {
                return Err(Stop::of(LOST).at(&named));
            };
            for &(normal, sheet_face) in &normals {
                let along = into.dot(normal);
                if along.is_nan() || along.abs() <= SQUARE {
                    return Err(Stop::of(TANGENT_CUT).at(&[named[0], sheet_face]));
                }
                let here = if along > 0.0 { 1 } else { -1 };
                let group = root(&mut parents, r);
                match side.get(&group) {
                    None => {
                        side.insert(group, here);
                    }
                    Some(&there) if there == here => {}
                    Some(_) => return Err(Stop::of(BOTH_SIDES).at(&[named[0], sheet_face])),
                }
            }
        }
    }
    let wanted = match op {
        BooleanOp::Subtract => 1,
        BooleanOp::Intersect => -1,
        BooleanOp::Union => return Err(Stop::of(super::SHEET_UNION)),
    };
    for (r, (face, loops)) in regions.into_iter().enumerate() {
        let group = root(&mut parents, r);
        let Some(&here) = side.get(&group) else {
            return Err(Stop::of(NOT_CROSSED).at(&[solid.name(face)]));
        };
        if here != wanted {
            continue;
        }
        let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
        kept.push(Kept {
            source: FaceSource {
                operand: solid.operand,
                face: face_id(face),
            },
            surface: f.surface.clone(),
            reversed: f.reversed,
            loops,
        });
    }
    Ok(())
}
