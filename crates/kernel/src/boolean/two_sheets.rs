//! A boolean of two sheets (decided by the developer, 2026-10-22).
//!
//! **Subtract and intersect: a sheet cut by a sheet**, as a solid is cut
//! by one ([`super::cut`]): subtract keeps the part of the target (A) in
//! front of the tool (B), the side the tool's normal points to; intersect
//! the part behind. The target's faces are divided where the tool passes
//! through them, and the regions are told apart by those curves alone:
//! along such a curve the region of the target beside it is in front of
//! the tool when the direction from the curve into the region, in the
//! target's surface, points the way the tool's normal points, and behind
//! when it points against it. Regions that share an arc the tool does not
//! meet are on one side (a flood over arcs). The tool must reach across
//! the target: a curve it crosses the target in that ends inside a face
//! (the tool's boundary passes through the target there) divides nothing
//! and is refused, naming the faces; so is a region no meeting reaches, a
//! region reached from both sides, a tool tangent to the target along a
//! curve, and a tool that lies on the target's surface (nothing there
//! tells front from behind). The result is a sheet whose faces are pieces
//! of the target's, facing as they did.
//!
//! **Union: two sheets sewn** along boundary edges they share (within the
//! tolerance: the pieces of their edges are one arc where they coincide,
//! so the sewn edge keeps the name of the target's edge). Where they meet
//! anywhere else (one passes through the other, ends on it, or lies over
//! it on one surface) the union is refused with the reason; so is a shared
//! edge along which the two sheets face opposite ways (no orientation
//! holds both), and one that three faces would use.

use super::classify::{FaceRegions, Kept, boundary, face_regions, on_coincident};
use super::outline::{locate, outward_normal};
use super::regions::Half;
use super::section::{Solid, Stretch};
use super::sew::{root, unite};
use super::{Arcs, BooleanOp, FaceSource, LOST, Operand, Reason, Stop, Vertices, face_id};
use craft3d_math::Tolerance;
use craft3d_topo::FaceId;
use std::collections::{BTreeMap, BTreeSet};

/// The tool sheet ends inside the target.
pub(super) const TOOL_ENDS_INSIDE: Reason = "the tool sheet ends inside the target sheet: to cut a sheet the tool must reach across it, its boundary outside the target or on the target's boundary";
/// The tool stands on the target: its boundary lies across it.
pub(super) const STANDS_ON: Reason = "the tool sheet stands on the target sheet: its boundary lies across the target, so it does not reach through it; to cut a sheet the tool must pass through it, its boundary beyond the target";
/// The tool divides no face of the target.
pub(super) const NOT_CUT: Reason = "the tool sheet does not pass through the target sheet: it misses it, or meets it only where the whole target is on one side of it, so it cuts nothing";
/// A region of the target is reached from both sides of the tool.
pub(super) const BOTH_SIDES: Reason = "the tool sheet does not divide the target sheet: a part of the target lies on both sides of the tool";
/// A part of the target the tool does not cross.
pub(super) const NOT_CROSSED: Reason = "a part of the target sheet that the tool sheet does not cross: which side of an open sheet it lies on is not told (cut a sheet of one part, or cut its parts one by one)";
/// The tool meets the target tangentially along a curve.
pub(super) const TANGENT: Reason = "the tool sheet meets the target sheet tangentially along a curve: which side is which cannot be told there";
/// The tool lies on the target's surface.
pub(super) const ON_SURFACE: Reason = "the tool sheet lies on the surface of the target sheet where they meet: a sheet is cut by a sheet that passes through it, and on one surface nothing tells front from behind";
/// Two sheets of a union meet other than along shared boundary edges.
pub(super) const UNION_CROSS: Reason = "a union of two sheets sews them along boundary edges they share; here they meet inside a face of one of them (one passes through the other or ends on it): cut one by the other first";
/// Two sheets of a union overlap on one surface.
pub(super) const UNION_OVERLAP: Reason = "a union of two sheets sews them along boundary edges they share; here they overlap on one surface";
/// Two sheets of a union face opposite ways along an edge they share.
pub(super) const UNION_OPPOSED: Reason = "the two sheets face opposite ways across an edge they share: a sewn sheet has one side, so make one of them face the other way first";
/// A union that closes some shells and leaves others open.
pub(super) const UNION_PART_CLOSED: Reason = "the union closes some of the sheets into a closed shell and leaves others open: a body is a solid of closed shells or a sheet of open ones, not both";
/// A union that closes into a shell facing inward.
pub(super) const UNION_INWARD: Reason = "the sewn sheets close into a shell that faces inward (it encloses no positive volume): a solid's faces look out of it, so make the sheets face the other way first";
/// A union that closes into a shell whose volume cannot be measured.
pub(super) const UNION_UNMEASURED: Reason = "the sewn sheets close into a shell whose volume could not be measured, so whether it faces out (a solid) is not told";
/// An edge of a union that three or more faces would use.
pub(super) const UNION_BRANCH: Reason = "more than two faces of the two sheets meet along an edge: a sheet's edge is used by one face or by two";

/// Below this the direction into a region and the tool's normal are
/// taken to be square to each other (as a sine), as in [`super::cut`].
const SQUARE: f64 = 1e-6;

/// The faces named by a stretch of the section: A's and B's.
fn named(a: &Solid, b: &Solid, stretch: &Stretch) -> [(Operand, FaceId); 2] {
    [a.name(stretch.fa), b.name(stretch.fb)]
}

/// The regions of the target sheet `a` on the side of the tool sheet `b`
/// that `op` keeps (subtract: in front, intersect: behind), added to
/// `kept`. `met` lists, per arc of the section, the stretches (by index
/// into `stretches`) it was made from.
#[allow(clippy::too_many_arguments)]
pub(super) fn cut(
    a: &Solid,
    b: &Solid,
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    stretches: &[Stretch],
    met: &BTreeMap<usize, Vec<usize>>,
    arcs: &Arcs,
    vertices: &Vertices,
    op: BooleanOp,
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let wanted: i8 = match op {
        BooleanOp::Subtract => 1,
        BooleanOp::Intersect => -1,
        BooleanOp::Union => return Err(Stop::of(LOST)),
    };
    if let Some((&fa, on)) = coincident.iter().next() {
        let fb = on.first().map_or(0, |&(fb, _)| fb);
        return Err(Stop::of(ON_SURFACE).at(&[a.name(fa), b.name(fb)]));
    }
    // The regions of every face of the target, each with its face.
    let b_box = b.bbox();
    let none = BTreeMap::new();
    let mut regions: Vec<(usize, Vec<Vec<Half>>)> = Vec::new();
    // Curves the tool crosses a face in that divide nothing, with the face.
    let mut dropped: Vec<(usize, usize)> = Vec::new();
    for face in 0..a.faces() {
        let FaceRegions { regions: found, .. } = face_regions(
            a,
            face,
            pieces,
            interior,
            &none,
            &[],
            arcs,
            vertices,
            &b_box,
            tol,
        )?;
        // A curve the tool crosses this face in that no region runs along
        // divides nothing (told below, once every face is divided).
        if let Some(across) = interior.get(&face) {
            let used: BTreeSet<usize> = found.iter().flatten().flatten().map(|h| h.0).collect();
            dropped.extend(
                across
                    .iter()
                    .filter(|arc| !used.contains(arc))
                    .map(|&arc| (arc, face)),
            );
        }
        regions.extend(found.into_iter().map(|loops| (face, loops)));
    }
    // Per arc, its uses by the regions.
    let mut by_region: BTreeMap<usize, Vec<(usize, bool)>> = BTreeMap::new();
    for (r, (_, loops)) in regions.iter().enumerate() {
        for &(arc, reversed) in loops.iter().flatten() {
            by_region.entry(arc).or_default().push((r, reversed));
        }
    }
    // Where the tool meets the target with its own boundary alone (one
    // face of the tool, every stretch along that face's boundary): across
    // the target (inside a face, or along an edge two faces of it share)
    // the tool stands on the target and does not reach through it; along
    // the target's boundary it only touches it there, which tells nothing.
    let mut touching: BTreeSet<usize> = BTreeSet::new();
    for (&arc, from) in met {
        let of: Vec<&Stretch> = from.iter().filter_map(|&s| stretches.get(s)).collect();
        let tool_faces: BTreeSet<usize> = of.iter().map(|s| s.fb).collect();
        if tool_faces.len() != 1 || of.iter().any(|s| s.inside_b) {
            continue;
        }
        let across = interior.values().any(|set| set.contains(&arc))
            || by_region.get(&arc).is_some_and(|uses| uses.len() >= 2);
        if across {
            let at = of
                .first()
                .map_or_else(Vec::new, |s| named(a, b, s).to_vec());
            return Err(Stop::of(STANDS_ON).at(&at));
        }
        touching.insert(arc);
    }
    // A curve that divides nothing: where one of its ends lies on the
    // tool's boundary and not on the target's, the tool's boundary passes
    // through the target there and the tool does not reach across it;
    // otherwise it runs from boundary to boundary without dividing (a
    // ring cut once): a part of the target is on both sides of the tool.
    if let Some(&(loose, face)) = dropped.first() {
        let at = met
            .get(&loose)
            .and_then(|s| s.first())
            .and_then(|&s| stretches.get(s))
            .map_or_else(|| vec![a.name(face)], |s| named(a, b, s).to_vec());
        let (rim_a, rim_b) = (rim_edges(a), rim_edges(b));
        let mut ends_inside = false;
        for &(arc, _) in &dropped {
            let arc = arcs.arcs.get(arc).ok_or(Stop::of(LOST))?;
            for v in [arc.start, arc.end] {
                let p = *vertices.points.get(v).ok_or(Stop::of(LOST))?;
                if on_rim(b, &rim_b, p, tol) && !on_rim(a, &rim_a, p, tol) {
                    ends_inside = true;
                }
            }
        }
        let reason = if ends_inside {
            TOOL_ENDS_INSIDE
        } else {
            BOTH_SIDES
        };
        return Err(Stop::of(reason).at(&at));
    }
    // The tool divides the target somewhere: an arc it meets the target
    // along with two regions beside it.
    let divides = met.keys().any(|arc| {
        !touching.contains(arc) && by_region.get(arc).is_some_and(|uses| uses.len() >= 2)
    });
    if !divides {
        return Err(Stop::of(NOT_CUT));
    }
    // Regions joined by an arc the tool does not meet are on one side.
    let mut parents: Vec<usize> = (0..regions.len()).collect();
    for (arc, uses) in &by_region {
        if met.contains_key(arc) {
            continue;
        }
        if let Some(&(first, _)) = uses.first() {
            for &(other, _) in uses.iter().skip(1) {
                unite(&mut parents, first, other);
            }
        }
    }
    // The side of each group, from every arc the tool meets the target
    // along: +1 in front of the tool, −1 behind.
    let mut side: BTreeMap<usize, i8> = BTreeMap::new();
    // Whether the tool passes through the target anywhere: an arc with
    // the target on both sides of it.
    let mut crossed = false;
    for (arc_index, from) in met {
        if touching.contains(arc_index) {
            continue;
        }
        let Some(uses) = by_region.get(arc_index) else {
            continue;
        };
        let mut read = [false; 2];
        let arc = arcs.arcs.get(*arc_index).ok_or(Stop::of(LOST))?;
        let middle = 0.5 * (arc.range.0 + arc.range.1);
        let (point, tangent) = arc.curve.derivatives(middle);
        // The tool's normal there, from every face of the tool the arc
        // was met along (two where an edge of a folded tool runs through
        // the target).
        let tool_faces: BTreeSet<usize> = from
            .iter()
            .filter_map(|&s| stretches.get(s))
            .map(|s| s.fb)
            .collect();
        let mut normals = Vec::with_capacity(tool_faces.len());
        for &face in &tool_faces {
            let outline = b.outline(face)?;
            let normal = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&[b.name(face)]))?;
            normals.push((normal, face));
        }
        for &(r, reversed) in uses {
            let (face, _) = regions.get(r).ok_or(Stop::of(LOST))?;
            let here_named = a.name(*face);
            let outline = a.outline(*face)?;
            let normal = outward_normal(outline.sheet(), outline.reversed, point)
                .ok_or(Stop::of(LOST).at(&[here_named]))?;
            let travel = if reversed { -tangent } else { tangent };
            // Seen from the side the face looks to, it lies to the left
            // of its loops.
            let Some(into) = normal.cross(travel).normalized() else {
                return Err(Stop::of(LOST).at(&[here_named]));
            };
            for &(tool_normal, tool_face) in &normals {
                let along = into.dot(tool_normal);
                let both = [here_named, b.name(tool_face)];
                if along.is_nan() {
                    return Err(Stop::of(LOST).at(&both));
                }
                if along.abs() <= SQUARE {
                    return Err(Stop::of(TANGENT).at(&both));
                }
                let here = if along > 0.0 { 1 } else { -1 };
                if let Some(seen) = read.get_mut(usize::from(here > 0)) {
                    *seen = true;
                }
                let group = root(&mut parents, r);
                match side.get(&group) {
                    None => {
                        side.insert(group, here);
                    }
                    Some(&there) if there == here => {}
                    Some(_) => return Err(Stop::of(BOTH_SIDES).at(&both)),
                }
            }
        }
        crossed |= read == [true, true];
    }
    // The tool meets the target only where the whole target is on one
    // side of it (through a fold of the target, beside it): it cuts
    // nothing.
    if !crossed {
        return Err(Stop::of(NOT_CUT));
    }
    for (r, (face, loops)) in regions.into_iter().enumerate() {
        let group = root(&mut parents, r);
        let Some(&here) = side.get(&group) else {
            return Err(Stop::of(NOT_CROSSED).at(&[a.name(face)]));
        };
        if here != wanted {
            continue;
        }
        let f = a.body.faces.get(face).ok_or(Stop::of(LOST))?;
        kept.push(Kept {
            source: FaceSource {
                operand: a.operand,
                face: face_id(face),
            },
            surface: f.surface.clone(),
            reversed: f.reversed,
            loops,
        });
    }
    Ok(())
}

/// The edges of a body that one face uses (a sheet's boundary).
fn rim_edges(solid: &Solid) -> BTreeSet<usize> {
    let mut uses: BTreeMap<usize, usize> = BTreeMap::new();
    for face in &solid.body.faces {
        for loop_ in face.loops.iter().filter_map(|&l| solid.body.loop_(l)) {
            for coedge in &loop_.coedges {
                *uses.entry(coedge.edge.0 as usize).or_default() += 1;
            }
        }
    }
    uses.into_iter()
        .filter(|&(_, n)| n == 1)
        .map(|(edge, _)| edge)
        .collect()
}

/// Whether `p` lies within the tolerance of an edge in `rim`.
fn on_rim(solid: &Solid, rim: &BTreeSet<usize>, p: craft3d_math::Vec3, tol: f64) -> bool {
    rim.iter().any(|&index| {
        let id = craft3d_topo::EdgeId(u32::try_from(index).unwrap_or(u32::MAX));
        solid.edge_box(id).contains(p, tol)
            && solid
                .body
                .edges
                .get(index)
                .and_then(|edge| locate(&edge.curve, edge.range, p, tol))
                .is_some()
    })
}

/// Every face of both sheets, whole, to be sewn along the boundary edges
/// they share; refused where they meet otherwise.
#[allow(clippy::too_many_arguments)]
pub(super) fn union(
    a: &Solid,
    b: &Solid,
    pieces_a: &[Vec<Half>],
    pieces_b: &[Vec<Half>],
    coincident_a: &BTreeMap<usize, Vec<(usize, bool)>>,
    stretches: &[Stretch],
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    // Where they meet inside a face of either: they cross there, one ends
    // on the other, or one's boundary lies over the other on one surface.
    for stretch in stretches {
        if stretch.inside_a || stretch.inside_b {
            let on_one = coincident_a
                .get(&stretch.fa)
                .is_some_and(|on| on.iter().any(|&(fb, _)| fb == stretch.fb));
            let reason = if on_one { UNION_OVERLAP } else { UNION_CROSS };
            return Err(Stop::of(reason).at(&named(a, b, stretch)));
        }
    }
    // Faces on one surface whose boundaries do not cross: they share
    // edges only, or one lies over the other (the same face twice, or
    // their boundaries along each other): a point inside the face tells.
    for (&fa, on) in coincident_a {
        let outline = a.outline(fa)?;
        if let Some((_, fb)) = on_coincident(outline, b, on, tolerance)? {
            return Err(Stop::of(UNION_OVERLAP).at(&[a.name(fa), b.name(fb)]));
        }
    }
    for (solid, pieces) in [(a, pieces_a), (b, pieces_b)] {
        for face in 0..solid.faces() {
            let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
            kept.push(Kept {
                source: FaceSource {
                    operand: solid.operand,
                    face: face_id(face),
                },
                surface: f.surface.clone(),
                reversed: f.reversed,
                loops: boundary(solid, face, pieces)?,
            });
        }
    }
    // Every arc is used by one face, or by two running it opposite ways.
    let mut uses: BTreeMap<usize, Vec<(bool, FaceSource)>> = BTreeMap::new();
    for k in kept.iter() {
        for &(arc, reversed) in k.loops.iter().flatten() {
            uses.entry(arc).or_default().push((reversed, k.source));
        }
    }
    for of_arc in uses.values() {
        let at: Vec<_> = of_arc.iter().map(|(_, s)| (s.operand, s.face)).collect();
        match of_arc.as_slice() {
            [_] => {}
            [(first, _), (second, _)] if first != second => {}
            [(_, x), (_, y)] if x.operand != y.operand => {
                return Err(Stop::of(UNION_OPPOSED).at(&at));
            }
            // Two faces of one sheet already sharing the edge the same
            // way: the operand was not a valid sheet.
            [_, _] => return Err(Stop::of(LOST).at(&at)),
            _ => return Err(Stop::of(UNION_BRANCH).at(&at)),
        }
    }
    Ok(())
}

/// A union of two sheets whose result has no boundary left (six faces of
/// a box sewn one by one): a closed shell that faces out encloses a
/// volume and is a solid's shell, so the body becomes a solid, its faces,
/// edges and vertices (and their names) as sewn. Refused when some shells
/// close and others do not, or when a closed shell faces inward (its
/// volume not more than the integrator's bound).
pub(super) fn closed_union(body: &mut craft3d_topo::Body) -> Result<(), Stop> {
    let closed: Vec<bool> = (0..body.shells.len())
        .map(|s| {
            let id = craft3d_topo::ShellId(u32::try_from(s).unwrap_or(u32::MAX));
            craft3d_topo::boundary_loops(body, id) == Some(0)
        })
        .collect();
    if !closed.iter().any(|&c| c) {
        return Ok(());
    }
    if !closed.iter().all(|&c| c) {
        return Err(Stop::of(UNION_PART_CLOSED));
    }
    for shell in craft3d_topo::shell_volumes(body) {
        let Ok(shell) = shell else {
            return Err(Stop::of(UNION_UNMEASURED));
        };
        // (A NaN volume is no positive volume either.)
        let outward = shell.volume > shell.bound.max(0.0);
        if !outward {
            return Err(Stop::of(UNION_INWARD));
        }
    }
    body.kind = craft3d_topo::BodyKind::Solid;
    Ok(())
}
