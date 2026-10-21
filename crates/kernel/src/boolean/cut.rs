//! A solid cut by a sheet: which regions of the solid's faces lie in
//! front of the sheet and which behind it.
//!
//! A sheet has no inside, so the regions of the solid are not classified
//! by rays against it. They are told apart by where the sheet meets the
//! solid's boundary: along the boundary of the sheet's piece inside the
//! solid (the *cap*, the sheet's regions that lie inside the solid,
//! classified by rays as any region is), where the sheet passes through
//! the boundary, and along the boundary of the solid's regions that lie
//! on the sheet, where the sheet's contact with a face ends. Along such an
//! arc the region of a face of the solid beside it is in front of the
//! sheet when the direction from the arc into the region points the way
//! the sheet's normal points, and behind when it points against it.
//! Regions that share an arc that is no such meeting are on one side.
//! The sheet has to reach across the solid: every arc of the cap's
//! boundary lies on the solid's boundary, every region is reached from a
//! meeting, and no region is on both sides; otherwise the cut is refused
//! with the reason.
//!
//! A region of the solid that lies on the sheet (a piece of the sheet on
//! a face of the solid, which counts as inside the solid: decided
//! 2026-10-18) is on neither side: it is a face that the solid and the
//! material behind the sheet share, and it is kept or not as the table of
//! two solids keeps a coincident face of A (`classify::solids`: intersect
//! keeps it when it looks the sheet's way, subtract when it looks the
//! other way). It is told by its class, not by its arcs, and no side is
//! carried across it. A region beside it that continues the sheet's
//! surface (a face of the solid beyond the sheet's rim, a face tangent to
//! the sheet there) is told nothing by that arc: it is reached from
//! another meeting, or not at all.

use super::classify::{
    Class, FaceRegions, Kept, face_regions, on_coincident, region_outline, solids,
};
use super::outline::outward_normal;
use super::regions::Half;
use super::section::Solid;
use super::sew::{root, unite};
use super::{Arcs, BooleanOp, FaceSource, LOST, Reason, Stop, Vertices, face_id};
use craft3d_math::Tolerance;
use std::collections::{BTreeMap, BTreeSet};

/// The sheet ends inside the solid.
pub(super) const SHEET_ENDS_INSIDE: Reason = "the sheet ends inside the solid: to cut a solid a sheet must reach across it, its boundary outside the solid or on its faces";
/// No piece of the sheet lies inside the solid or on its boundary.
pub(super) const NOT_CUT: Reason = "the sheet does not pass through the solid: no piece of it lies inside the solid or on its faces, so it cuts nothing";
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

/// A region of a face of the solid: the face, its loops of arcs, and its
/// class and the face of the sheet it lies on when it lies on the sheet
/// (`None` beside the sheet, where its arcs tell its side).
struct Region {
    face: usize,
    loops: Vec<Vec<Half>>,
    on_sheet: Option<(Class, usize)>,
}

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
    let mut regions: Vec<Region> = Vec::new();
    for face in 0..solid.faces() {
        let named = [solid.name(face)];
        let coincident = coincident.get(&face).map_or(&[][..], Vec::as_slice);
        // No curve is held to dividing a face here: every curve the sheet
        // crosses a face of the solid in is a cut, and one that divides
        // nothing (a ring cut once) is told below as a sheet that does not
        // divide the solid.
        let FaceRegions {
            regions: found,
            whole,
            outline,
            ..
        } = face_regions(
            solid, face, pieces, interior, &none, coincident, arcs, vertices, &sheet_box, tol,
        )?;
        for loops in found {
            // A region of a face on the sheet's surface lies on the sheet
            // or beside it.
            let mut on_sheet = None;
            if let Some(outline) = outline.filter(|_| !coincident.is_empty()) {
                let made = region_outline(outline, whole, &loops, arcs, vertices, tol, &named)?;
                let region = made.as_ref().unwrap_or(outline);
                on_sheet = on_coincident(region, sheet, coincident, tolerance)
                    .map_err(|s| s.at(&named))?;
            }
            regions.push(Region {
                face,
                loops,
                on_sheet,
            });
        }
    }
    if cap.is_empty() && regions.iter().all(|r| r.on_sheet.is_none()) {
        return Err(Stop::of(NOT_CUT));
    }
    // Where the sheet meets the solid's boundary: per arc, the faces of
    // the sheet whose normal is read there. The cap's boundary (the sheet
    // passes through the boundary), and the boundary of the solid's
    // regions on the sheet (its contact with a face ends).
    let mut meetings: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (arc, caps) in &by_cap {
        for &c in caps {
            let piece = cap.get(c).ok_or(Stop::of(LOST))?;
            meetings
                .entry(*arc)
                .or_default()
                .insert(piece.source.face.0 as usize);
        }
    }
    for region in &regions {
        let Some((_, sheet_face)) = region.on_sheet else {
            continue;
        };
        for &(arc, _) in region.loops.iter().flatten() {
            meetings.entry(arc).or_default().insert(sheet_face);
        }
    }
    // Per arc, its uses by the regions of the solid beside the sheet.
    let mut by_solid: BTreeMap<usize, Vec<(usize, bool)>> = BTreeMap::new();
    for (r, region) in regions.iter().enumerate() {
        if region.on_sheet.is_some() {
            continue;
        }
        for &(arc, reversed) in region.loops.iter().flatten() {
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
    // Regions joined by an arc where the sheet meets nothing are on one
    // side.
    let mut parents: Vec<usize> = (0..regions.len()).collect();
    for (arc, uses) in &by_solid {
        if meetings.contains_key(arc) {
            continue;
        }
        if let Some(&(first, _)) = uses.first() {
            for &(other, _) in uses.iter().skip(1) {
                unite(&mut parents, first, other);
            }
        }
    }
    // The side of each group, from every meeting that bounds a region of
    // it: +1 in front of the sheet, −1 behind.
    let mut side: BTreeMap<usize, i8> = BTreeMap::new();
    for (arc_index, sheet_faces) in &meetings {
        let Some(uses) = by_solid.get(arc_index) else {
            continue;
        };
        let arc = arcs.arcs.get(*arc_index).ok_or(Stop::of(LOST))?;
        let middle = 0.5 * (arc.range.0 + arc.range.1);
        let (point, tangent) = arc.curve.derivatives(middle);
        // The sheet's own normal there (not the cap's, which subtract has
        // turned over), from every face of the sheet along the arc.
        let mut normals = Vec::with_capacity(sheet_faces.len());
        for &face in sheet_faces {
            let outline = sheet.outline(face)?;
            let normal = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&[sheet.name(face)]))?;
            normals.push((normal, face));
        }
        let through = by_cap.contains_key(arc_index);
        for &(r, reversed) in uses {
            let region = regions.get(r).ok_or(Stop::of(LOST))?;
            let named = [solid.name(region.face)];
            let outline = solid.outline(region.face)?;
            let outward = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&named))?;
            let travel = if reversed { -tangent } else { tangent };
            // Seen from outside the face lies to the left of its loops.
            let Some(into) = outward.cross(travel).normalized() else {
                return Err(Stop::of(LOST).at(&named));
            };
            // The faces of the sheet whose surface this region's face
            // lies on.
            let level = coincident.get(&region.face).map_or(&[][..], Vec::as_slice);
            for &(normal, face) in &normals {
                let sheet_face = sheet.name(face);
                let along = into.dot(normal);
                if along.is_nan() {
                    return Err(Stop::of(LOST).at(&[named[0], sheet_face]));
                }
                if along.abs() <= SQUARE {
                    // A region on the surface of this face of the sheet
                    // (beside the piece of the sheet on it: where the
                    // contact ends, or a sheet that folds there) continues
                    // the sheet's surface and is told nothing here: it is
                    // reached from another meeting, or not at all. Any
                    // other region square to the sheet where the sheet
                    // passes through the boundary is a cut tangent to a
                    // face.
                    if through && !level.iter().any(|&(f, _)| f == face) {
                        return Err(Stop::of(TANGENT_CUT).at(&[named[0], sheet_face]));
                    }
                    continue;
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
    for (r, region) in regions.into_iter().enumerate() {
        let Region {
            face,
            loops,
            on_sheet,
        } = region;
        let flip = match on_sheet {
            // On the sheet: as a face of A coincident with a face of B.
            Some((class, _)) => match solids(op, solid.operand, class)
                .map_err(|reason| Stop::of(reason).at(&[solid.name(face)]))?
            {
                Some(flip) => flip,
                None => continue,
            },
            None => {
                let group = root(&mut parents, r);
                let Some(&here) = side.get(&group) else {
                    return Err(Stop::of(NOT_CROSSED).at(&[solid.name(face)]));
                };
                if here != wanted {
                    continue;
                }
                false
            }
        };
        let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
        let loops = if flip {
            loops
                .into_iter()
                .map(|l| l.into_iter().rev().map(|(arc, r)| (arc, !r)).collect())
                .collect()
        } else {
            loops
        };
        kept.push(Kept {
            source: FaceSource {
                operand: solid.operand,
                face: face_id(face),
            },
            surface: f.surface.clone(),
            reversed: f.reversed != flip,
            loops,
        });
    }
    Ok(())
}
