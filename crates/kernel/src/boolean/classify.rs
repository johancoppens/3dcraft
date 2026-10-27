//! Where the regions of one solid's faces are with respect to the other
//! solid, and which of them the operation keeps.

use super::hits::{curve_hits, signed_distance};
use super::outline::{
    Outline, SLIVER, Side, boxes_meet, line_in_box, nearest, onto, outward_normal, surface_distance,
};
use super::regions::{self, ANGLE_GUARD, Half};
use super::section::Solid;
use super::{
    Arcs, BooleanOp, FaceSource, IMPRINT_LOST, LOST, NO_RAY, ON_A_FACE, Operand, Reason, Stop,
    Vertices,
};
use super::{face_id, geometry};
use craft3d_geom::intersect::CurveIntersection;
use craft3d_geom::{Curve3, Line, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Coedge;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    In,
    Out,
    /// On a face of the other solid whose outward normal is the same.
    OnSame,
    /// On a face of the other solid that faces the other way.
    OnOpposite,
}

/// Directions for the rays of [`inside`]: none along an axis or a diagonal.
const RAYS: [(f64, f64, f64); 12] = [
    (0.577, 0.211, 0.789),
    (-0.343, 0.871, 0.351),
    (0.129, -0.497, 0.858),
    (-0.811, -0.263, 0.523),
    (0.657, 0.731, -0.183),
    (0.279, -0.913, -0.297),
    (-0.593, 0.437, -0.677),
    (0.913, -0.151, 0.379),
    (-0.117, 0.337, -0.934),
    (0.453, 0.619, 0.641),
    (-0.767, -0.587, -0.259),
    (0.211, -0.143, -0.967),
];

/// How many points of a region are tried before giving up on telling where
/// it is.
const TRIES: usize = 4;

/// True when `p`, which is not on its boundary, is inside `solid`: a ray
/// from it crosses the boundary an odd number of times. A ray that meets
/// an edge, a vertex, or a face tangentially is abandoned for the next.
pub(super) fn inside(solid: &Solid, p: Vec3, tolerance: &Tolerance) -> Result<bool, Stop> {
    let tol = tolerance.linear;
    // A point within reach of a face of the solid is neither inside nor
    // outside it by any margin that means something.
    let mut tested = crate::work::Tally::faces();
    for face in solid.candidates(|b| b.contains(p, 2.0 * tol)) {
        tested.one();
        let Some(box_) = solid.boxes.get(face) else {
            return Err(Stop::of(LOST));
        };
        if !box_.contains(p, 2.0 * tol) {
            continue;
        }
        let outline = solid.outline(face)?;
        // A bound below the distance to a freeform face (a few boxes)
        // settles most points before a nearest-point search.
        let sheet = outline.sheet();
        let far = sheet
            .tree
            .is_some_and(|tree| tree.lower_bound(p, tol, 2.001 * tol) >= 2.0 * tol);
        let near = !far && signed_distance(sheet, p).is_some_and(|d| d.abs() < 2.0 * tol);
        if near && let Ok((u, v)) = outline.surface.project(p) {
            let named = [solid.name(face)];
            let foot = outline.surface.eval(u, v);
            if outline
                .side(foot, tol)
                .map_err(|r| Stop::of(r).at(&named))?
                != Side::Out
            {
                return Err(Stop::of(ON_A_FACE).at(&named));
            }
        }
    }
    'rays: for (x, y, z) in RAYS {
        let Ok(line) = Line::new(p, Vec3::new(x, y, z)) else {
            continue;
        };
        let ray = Curve3::Line(line);
        let mut crossings = 0_usize;
        // Behind the point there is nothing to count. The stretch
        // looked at starts at the point and ends well beyond the
        // face, so that neither end of it is a place where the ray
        // is found near the surface without having crossed it.
        let from = -2.0 * tol;
        // The faces whose boxes the ray may meet ahead of the point, in
        // the order of the faces (a node passes unless the ray misses it
        // or leaves it behind the point).
        let (origin, direction) = (line.origin(), line.direction());
        let reach = solid.candidates(|b| {
            line_in_box(origin, direction, b, 2.0 * tol)
                .is_some_and(|(_, leave)| leave.is_nan() || leave > from)
        });
        for face in reach {
            tested.one();
            let Some(box_) = solid.boxes.get(face) else {
                return Err(Stop::of(LOST));
            };
            let Some((enter, leave)) =
                line_in_box(line.origin(), line.direction(), box_, 2.0 * tol)
            else {
                continue;
            };
            if !(leave.is_finite() && leave > from) {
                continue;
            }
            let leave = leave + 16.0 * tol + 0.01 * (leave - enter.max(from));
            let outline = solid.outline(face)?;
            let named = [solid.name(face)];
            let side = |q: Vec3| outline.side(q, tol).map_err(|r| Stop::of(r).at(&named));
            let hits = match curve_hits(&ray, (from, leave), outline.sheet(), tolerance)
                .map_err(|e| geometry(e, &named))?
            {
                CurveIntersection::OnSurface => continue 'rays,
                CurveIntersection::Hits(hits) => hits,
            };
            for hit in hits {
                if hit.t.abs() <= tol {
                    // The point is on this surface: fine when the face is
                    // elsewhere on it.
                    if side(p)? != Side::Out {
                        return Err(Stop::of(ON_A_FACE).at(&named));
                    }
                    continue;
                }
                // (Further from the face than its cover is outside it.)
                if hit.t < 0.0
                    || !box_.contains(hit.point, 2.0 * tol)
                    || !solid.may_hold(face, hit.point, tol)
                {
                    continue;
                }
                match side(hit.point)? {
                    Side::Out => {}
                    Side::On => continue 'rays,
                    Side::In if hit.tangent => continue 'rays,
                    Side::In => crossings += 1,
                }
            }
        }
        return Ok(crossings % 2 == 1);
    }
    Err(Stop::of(NO_RAY))
}

/// How far from its boundary, in tolerances, a point that tells where a
/// region is must lie: away from the arcs along which the other body
/// passes through the face, which are on the other body.
const MARGIN: f64 = 4.0;

/// Up to `want` points of `region` to tell where it is by: at the margin
/// from its boundary, or, for a region that holds none there and that the
/// other body does not pass through (`crossed` is false: no arc of it is
/// on the other body, so it lies on one side of that body as a whole, and
/// any point of it that the ray classification accepts decides), as close
/// to its boundary as a sixteenth of the margin, the distance
/// [`outline::left_of`] reads a loop's witness at: which side of its own
/// loop a point is on is a question of the loop's pieces as they are, and
/// a point within the tolerance of the boundary is not taken
/// (`Outline::side` says `On` there). The point at the margin comes first,
/// so a region that one decides is decided as it was.
fn samples(region: &Outline, tol: f64, want: usize, crossed: bool) -> Result<Vec<Vec3>, Reason> {
    match region.interior_points(tol, MARGIN * tol, want) {
        Err(SLIVER) if !crossed => region.interior_points(tol, MARGIN * tol / 16.0, want),
        found => found,
    }
}

/// Where a region of a face of `own` is with respect to `other`: by a
/// point well inside it, or, when the region holds no such point, read at
/// its boundary ([`at_boundary`]) where the other body passes through
/// it, and by a point closer to its boundary where it does not
/// ([`samples`]). `loops` are the region's loops of arcs, in the order of
/// `region`'s, and `across` the arcs that lie inside the face.
fn classify(
    region: &Outline,
    loops: &[Vec<Half>],
    across: &BTreeSet<usize>,
    other: &Solid,
    coincident: &[(usize, bool)],
    tolerance: &Tolerance,
) -> Result<Class, Stop> {
    let tol = tolerance.linear;
    let mut failure = None;
    // The other body passes through the face along an arc of this region:
    // the region is read at that arc when no point at the margin tells it.
    let crossed = loops.iter().flatten().any(|(arc, _)| across.contains(arc));
    // One point decides, nearly always: the others are looked for only
    // when it does not. (The points for a greater number begin with those
    // for a smaller one.)
    let mut tried = 0;
    'points: for want in [1, TRIES] {
        let candidates = match samples(region, tol, want, crossed) {
            Ok(candidates) => candidates,
            Err(SLIVER) if tried == 0 && crossed => {
                return at_boundary(region, loops, across, other, coincident, tolerance);
            }
            Err(reason) => return Err(Stop::of(reason)),
        };
        let fresh: Vec<Vec3> = candidates.into_iter().skip(tried).collect();
        tried += fresh.len();
        for p in fresh {
            if let Some((class, _)) = on_coincident_at(region, p, other, coincident, tol)? {
                return Ok(class);
            }
            // A point may by chance be where the bodies touch without
            // crossing, or have no usable ray: the next one decides as
            // well.
            match inside(other, p, tolerance) {
                Ok(true) => return Ok(Class::In),
                Ok(false) => return Ok(Class::Out),
                Err(stop @ Stop::Unsupported { .. }) => {
                    failure = failure.or(Some(stop));
                }
                Err(stop) => return Err(stop),
            }
        }
        if want == 1 && TRIES == 1 {
            break 'points;
        }
    }
    Err(failure.unwrap_or(Stop::of(NO_RAY)))
}

/// Whether the point `p` of a region of a face lies on a face of `other`
/// on the same surface (`coincident`): `Some(OnSame | OnOpposite)` and
/// that face when it is inside one of them, `None` when it is on none.
fn on_coincident_at(
    region: &Outline,
    p: Vec3,
    other: &Solid,
    coincident: &[(usize, bool)],
    tol: f64,
) -> Result<Option<(Class, usize)>, Stop> {
    for &(face, same_normal) in coincident {
        let outline = other.outline(face)?;
        // Coincident surfaces may overlap only in part: a point of
        // this one that is not on the other's (a rational patch
        // on the cylinder it is a quarter of, with a point of the
        // cylinder beyond the patch) is off the face, and has no
        // place in a chart that is taken from the patch.
        if surface_distance(outline.sheet(), p) > tol {
            continue;
        }
        match outline
            .side(p, tol)
            .map_err(|r| Stop::of(r).at(&[other.name(face)]))?
        {
            Side::Out => {}
            // A boundary of the other body that was not imprinted
            // here: no other point of the region would mean more.
            Side::On => return Err(Stop::of(ON_A_FACE).at(&[other.name(face)])),
            Side::In => {
                let same = same_normal == (region.reversed == outline.reversed);
                let class = if same {
                    Class::OnSame
                } else {
                    Class::OnOpposite
                };
                return Ok(Some((class, face)));
            }
        }
    }
    Ok(None)
}

/// Whether a region of a face of `own` lies on a face of `other` on the
/// same surface, told by a point well inside the region: `Some` with its
/// class and that face of `other` when it does, `None` when it does not, or when the region holds
/// no such point (a sliver, whose side the caller tells another way).
/// Where a face of `other` passes through the region, the imprint divides
/// it, so one point speaks for the whole region.
pub(super) fn on_coincident(
    region: &Outline,
    other: &Solid,
    coincident: &[(usize, bool)],
    tolerance: &Tolerance,
) -> Result<Option<(Class, usize)>, Stop> {
    if coincident.is_empty() {
        return Ok(None);
    }
    let tol = tolerance.linear;
    let candidates = match region.interior_points(tol, MARGIN * tol, 1) {
        Ok(candidates) => candidates,
        Err(SLIVER) => return Ok(None),
        Err(reason) => return Err(Stop::of(reason)),
    };
    for p in candidates {
        if let Some(found) = on_coincident_at(region, p, other, coincident, tol)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

const UNREAD: Reason = "a piece of a face too thin to find a point inside it, whose boundary does not say which side of the other body it is on (at the edge of tolerance)";
const DISAGREE: Reason = "a piece of a face too thin to find a point inside it, whose boundary reads as more than one side of the other body (at the edge of tolerance)";
/// The most arcs of a region that are read for its side at the boundary:
/// a sliver has a few; beyond this the region is not read.
const READ_ARCS: usize = 64;

/// Where a region that holds no point at the margin from its boundary (a
/// sliver a few tolerances wide: the corner a curve cuts off a face just
/// past its vertex, the triangle between three edges of three bodies
/// that nearly meet in one point) lies, read at its boundary, which is
/// exact where a sample point is not: just inside the region next to an
/// arc along which the other body passes through the face, the region is
/// where the direction into it points with respect to what the other
/// body is at that arc ([`beside`]). Every such arc of the region is read
/// and they must agree; a region without one is not read.
fn at_boundary(
    region: &Outline,
    loops: &[Vec<Half>],
    across: &BTreeSet<usize>,
    other: &Solid,
    coincident: &[(usize, bool)],
    tolerance: &Tolerance,
) -> Result<Class, Stop> {
    let mut found: Option<Class> = None;
    let mut read = 0;
    for (halves, segs) in loops.iter().zip(&region.loops) {
        for (&(arc, _), seg) in halves.iter().zip(segs) {
            if !across.contains(&arc) {
                continue;
            }
            if read >= READ_ARCS {
                return Err(Stop::of(UNREAD));
            }
            read += 1;
            // Into the region: seen from outside, it lies to the left of
            // its boundary.
            let (p, travel) = seg.travel();
            let Some(travel) = travel.normalized() else {
                continue;
            };
            let Some(outward) = outward_normal(region.sheet(), region.reversed, p) else {
                continue;
            };
            let Some(inward) = outward.cross(travel).normalized() else {
                continue;
            };
            let along = [seg.at(0.25), seg.at(0.75)];
            let class = beside(
                other,
                coincident,
                region,
                (p, along),
                travel,
                inward,
                tolerance,
            )?;
            match found {
                Some(before) if before != class => return Err(Stop::of(DISAGREE)),
                _ => found = Some(class),
            }
        }
    }
    found.ok_or(Stop::of(SLIVER))
}

/// Where the face of `region` is, just beside the point `p` of its
/// boundary in the direction `inward`, with respect to `other`, from the
/// faces of `other` that hold the arc there (`p` and the two places
/// `along` it, within the tolerance). `travel` is the direction of the
/// arc at `p`.
///
/// `p` is inside an arc, so it is inside a face of `other` or inside an
/// edge of it that runs along the arc: a face of `other` on the same
/// surface as the region's (`coincident`) that reaches to the inward side
/// has the region on it; otherwise the material of `other` next to `p`
/// lies behind the one face through `p`, or in the wedge of the two that
/// meet along the edge, which holds `inward` or not. A face through `p`
/// that does not hold the arc would make `p` a vertex, and a sign within
/// [`ANGLE_GUARD`] of zero means faces tangent there: neither is read.
fn beside(
    other: &Solid,
    coincident: &[(usize, bool)],
    region: &Outline,
    (p, along): (Vec3, [Vec3; 2]),
    travel: Vec3,
    inward: Vec3,
    tolerance: &Tolerance,
) -> Result<Class, Stop> {
    let tol = tolerance.linear;
    // The faces of the other body that are not on this surface and hold
    // the arc: their outward normals, and where `p` is on their boundary
    // the direction into them from it.
    let mut walls: Vec<(Vec3, Option<Vec3>)> = Vec::new();
    // A face on the region's surface that lies on the other side of its
    // edge along the arc: the second face of an edge that one wall is on.
    let mut far_side = false;
    for face in other.candidates(|b| b.contains(p, 2.0 * tol)) {
        let Some(box_) = other.boxes.get(face) else {
            return Err(Stop::of(LOST));
        };
        if !box_.contains(p, 2.0 * tol) {
            continue;
        }
        let outline = other.outline(face)?;
        let named = [other.name(face)];
        let sheet = outline.sheet();
        if surface_distance(sheet, p) > tol {
            continue;
        }
        let Some(foot) = onto(sheet, p) else {
            continue;
        };
        let side = outline
            .side(foot, tol)
            .map_err(|r| Stop::of(r).at(&named))?;
        if side == Side::Out {
            continue;
        }
        if along.iter().any(|&q| surface_distance(sheet, q) > tol) {
            return Err(Stop::of(UNREAD).at(&named));
        }
        let Some(normal) = outward_normal(sheet, outline.reversed, foot) else {
            return Err(Stop::of(UNREAD).at(&named));
        };
        let same_surface = coincident
            .iter()
            .find(|(f, _)| *f == face)
            .map(|&(_, same)| same);
        let on = |same_normal: bool| {
            if same_normal == (region.reversed == outline.reversed) {
                Class::OnSame
            } else {
                Class::OnOpposite
            }
        };
        if side == Side::In {
            match same_surface {
                Some(same_normal) => return Ok(on(same_normal)),
                None => walls.push((normal, None)),
            }
            continue;
        }
        // On the boundary of this face: along the piece of it that holds
        // the arc, the nearest such piece to `p`.
        let mut nearest_piece: Option<(f64, Vec3)> = None;
        for (l, k) in outline.pieces_near(foot, tol) {
            let Some(piece) = outline.loops.get(l).and_then(|loop_| loop_.get(k)) else {
                continue;
            };
            let Some((t, d)) = nearest(&piece.curve, piece.range, foot) else {
                continue;
            };
            if d.is_nan() || d > tol || nearest_piece.is_some_and(|(best, _)| d >= best) {
                continue;
            }
            let holds = along
                .iter()
                .all(|&q| nearest(&piece.curve, piece.range, q).is_some_and(|(_, d)| d <= tol));
            let span = piece.range.1 - piece.range.0;
            if !holds || span.is_nan() || span <= 0.0 {
                continue;
            }
            let (_, direction) = piece.travel_at((t - piece.range.0) / span);
            nearest_piece = Some((d, direction));
        }
        let Some((_, direction)) = nearest_piece else {
            return Err(Stop::of(UNREAD).at(&named));
        };
        let Some(direction) = direction.normalized() else {
            return Err(Stop::of(UNREAD).at(&named));
        };
        if direction.dot(travel).abs() < 0.5 {
            return Err(Stop::of(UNREAD).at(&named));
        }
        let Some(into) = normal.cross(direction).normalized() else {
            return Err(Stop::of(UNREAD).at(&named));
        };
        match same_surface {
            Some(same_normal) => {
                // Its own surface is the region's: it reaches to the
                // inward side, or lies on the other side of its edge.
                let reach = into.dot(inward);
                if reach > ANGLE_GUARD {
                    return Ok(on(same_normal));
                }
                if reach >= -ANGLE_GUARD {
                    return Err(Stop::of(UNREAD).at(&named));
                }
                far_side = true;
            }
            None => walls.push((normal, Some(into))),
        }
    }
    let inside = match walls.as_slice() {
        [] => return Err(Stop::of(UNREAD)),
        [(normal, on_edge)] => {
            // One face through `p`: the region is behind it or not. A
            // wall that has `p` on its boundary is one face of an edge
            // along the arc, and its other face must have been read
            // (the one on the region's surface, on the far side), or the
            // wedge is not known: a half-space would read a convex edge
            // as inside.
            if on_edge.is_some() && !far_side {
                return Err(Stop::of(UNREAD));
            }
            let d = inward.dot(*normal);
            if d.abs() <= ANGLE_GUARD {
                return Err(Stop::of(UNREAD));
            }
            d < 0.0
        }
        [(n1, w1), (n2, w2)] => {
            // An edge of the other body along the arc: its material is
            // the wedge between the two faces, less than a half turn
            // when each lies behind the other's surface.
            let (Some(w1), Some(w2)) = (w1, w2) else {
                return Err(Stop::of(UNREAD));
            };
            let (d1, d2) = (inward.dot(*n1), inward.dot(*n2));
            if d1.abs() <= ANGLE_GUARD || d2.abs() <= ANGLE_GUARD {
                return Err(Stop::of(UNREAD));
            }
            let (c1, c2) = (w1.dot(*n2), w2.dot(*n1));
            if c1.abs() <= ANGLE_GUARD && c2.abs() <= ANGLE_GUARD {
                // The faces continue each other smoothly: one wall.
                if (d1 < 0.0) != (d2 < 0.0) {
                    return Err(Stop::of(UNREAD));
                }
                d1 < 0.0
            } else if (c1 < 0.0) != (c2 < 0.0) {
                return Err(Stop::of(UNREAD));
            } else if c1 < 0.0 {
                d1 < 0.0 && d2 < 0.0
            } else {
                d1 < 0.0 || d2 < 0.0
            }
        }
        _ => return Err(Stop::of(UNREAD)),
    };
    Ok(if inside { Class::In } else { Class::Out })
}

/// A region of a face of an operand that becomes a face of the result.
pub(super) struct Kept {
    pub source: FaceSource,
    pub surface: Surface,
    pub reversed: bool,
    pub loops: Vec<Vec<Half>>,
}

/// The boundary of a face in arcs: per loop, the uses of the pieces of its
/// edges in the loop's direction.
pub(super) fn boundary(
    solid: &Solid,
    face: usize,
    pieces: &[Vec<Half>],
) -> Result<Vec<Vec<Half>>, Stop> {
    let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
    let mut loops = Vec::with_capacity(f.loops.len());
    for loop_ in f.loops.iter().filter_map(|&l| solid.body.loop_(l)) {
        let mut halves = Vec::new();
        for &Coedge { edge, reversed } in &loop_.coedges {
            let of_edge = pieces.get(edge.0 as usize).ok_or(Stop::of(LOST))?;
            if reversed {
                halves.extend(of_edge.iter().rev().map(|&(arc, flipped)| (arc, !flipped)));
            } else {
                halves.extend(of_edge.iter().copied());
            }
        }
        loops.push(halves);
    }
    Ok(loops)
}

/// What a selection keeps of a region of `own` in the place `class`:
/// `Ok(Some(flip))` keeps it (turned inside out when `flip`), `Ok(None)`
/// leaves it out, `Err` refuses the operation.
pub(super) type Table = fn(BooleanOp, Operand, Class) -> Result<Option<bool>, Reason>;

/// The table of two solids. Of two coincident regions with the same
/// normal A's is the one kept; with opposite normals the solids lie on
/// either side and only A minus B has a face there, A's.
pub(super) fn solids(op: BooleanOp, own: Operand, class: Class) -> Result<Option<bool>, Reason> {
    Ok(match (op, own, class) {
        (BooleanOp::Union, _, Class::Out) => Some(false),
        (BooleanOp::Union, Operand::A, Class::OnSame) => Some(false),
        (BooleanOp::Intersect, _, Class::In) => Some(false),
        (BooleanOp::Intersect, Operand::A, Class::OnSame) => Some(false),
        (BooleanOp::Subtract, Operand::A, Class::Out | Class::OnOpposite) => Some(false),
        (BooleanOp::Subtract, Operand::B, Class::In) => Some(true),
        _ => None,
    })
}

/// The table of a sheet's regions against a solid: the sheet is the
/// target (A) and is trimmed (intersect keeps what is inside the solid,
/// subtract what is outside), or the sheet is the tool (B) and its piece
/// inside the solid is the face where the solid is cut (turned inside
/// out for subtract, which keeps the side in front of the sheet).
///
/// A piece of the sheet lying on a face of the solid counts as inside
/// the solid, a closed set (decided 2026-10-18), whichever way the face
/// looks: a trim keeps it for intersect and takes it away for subtract,
/// so that `area(S ∩ A) + area(S ∖ A) = area(S)`. In a cut the sheet is
/// the boundary of the material behind it, and such a piece is a face
/// the solid and that material share: as between two solids ([`solids`])
/// the solid's own face stands for it there (kept or not by the solid's
/// table, [`super::cut`]), and the sheet's piece is never a cap.
pub(super) fn sheet(op: BooleanOp, own: Operand, class: Class) -> Result<Option<bool>, Reason> {
    Ok(match (class, op, own) {
        (Class::In | Class::OnSame | Class::OnOpposite, BooleanOp::Intersect, Operand::A) => {
            Some(false)
        }
        (Class::In, BooleanOp::Intersect, Operand::B) => Some(false),
        (Class::In, BooleanOp::Subtract, Operand::B) => Some(true),
        (Class::Out, BooleanOp::Subtract, Operand::A) => Some(false),
        _ => None,
    })
}

/// The outline of a region of a face whose outline is `outline`: `None`
/// when the region is the whole face (its own outline serves), else the
/// piece of the face bounded by `loops`.
pub(super) fn region_outline(
    outline: &Outline,
    whole: bool,
    loops: &[Vec<Half>],
    arcs: &Arcs,
    vertices: &Vertices,
    tol: f64,
    named: &[(Operand, craft3d_topo::FaceId)],
) -> Result<Option<Outline>, Stop> {
    if whole {
        return Ok(None);
    }
    let mut segs = Vec::with_capacity(loops.len());
    for loop_ in loops {
        segs.push(
            regions::segs(loop_, &arcs.arcs, &vertices.points)
                .map_err(|r| Stop::of(r).at(named))?,
        );
    }
    outline
        .piece(segs, MARGIN * tol)
        .map(Some)
        .map_err(|r| Stop::of(r).at(named))
}

/// The regions of face `face` of `own`, each as its loops of arcs, and
/// what is known of where they lie before classifying them: `apart` when
/// the face is whole and nowhere near `other_box` (so outside it).
pub(super) struct FaceRegions<'o> {
    pub regions: Vec<Vec<Vec<Half>>>,
    pub whole: bool,
    pub apart: bool,
    pub outline: Option<&'o Outline>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn face_regions<'o>(
    own: &'o Solid,
    face: usize,
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    crossing: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &[(usize, bool)],
    arcs: &Arcs,
    vertices: &Vertices,
    other_box: &craft3d_math::Bbox3,
    tol: f64,
) -> Result<FaceRegions<'o>, Stop> {
    let none = BTreeSet::new();
    let named = [own.name(face)];
    let loops = boundary(own, face, pieces)?;
    let across = interior.get(&face).unwrap_or(&none);
    let whole = across.is_empty()
        && own.edges_of(face).iter().all(|edge| {
            pieces
                .get(edge.0 as usize)
                .is_some_and(|halves| halves.len() == 1)
        });
    // A whole face that is nowhere near the other solid's box is
    // outside that solid, without looking for a point of it (and
    // without its outline).
    let apart = whole
        && coincident.is_empty()
        && own
            .boxes
            .get(face)
            .is_some_and(|own_box| !boxes_meet(own_box, other_box, 4.0 * tol));
    let outline = if apart {
        None
    } else {
        Some(own.outline(face)?)
    };
    let regions = if whole {
        vec![loops]
    } else {
        let outline = outline.ok_or(Stop::of(LOST))?;
        let (regions, dropped) = regions::build(
            outline.sheet(),
            outline.reversed,
            &loops,
            across,
            &arcs.arcs,
            &vertices.points,
            tol,
        )
        .map_err(|r| Stop::of(r).at(&named))?;
        // Where the other body's face passes through this one, what
        // is on one side of the curve is inside that body and what is
        // on the other side is not: such a curve divides the face, or
        // a piece of the imprint is missing.
        if crossing
            .get(&face)
            .is_some_and(|must| !must.is_disjoint(&dropped))
        {
            if let Some(must) = crossing.get(&face) {
                for &i in must.intersection(&dropped) {
                    if let Some(arc) = arcs.arcs.get(i) {
                        let (p, q) = (arc.curve.eval(arc.range.0), arc.curve.eval(arc.range.1));
                        eprintln!(
                            "BOOLDBG imprint lost face {face} arc {i} from {p:?} to {q:?} range {:?} vertices {} {} at {:?} {:?} across {} dropped {}",
                            arc.range,
                            arc.start,
                            arc.end,
                            vertices.points.get(arc.start),
                            vertices.points.get(arc.end),
                            across.len(),
                            dropped.len()
                        );
                    }
                }
                for &i in across {
                    if let Some(arc) = arcs.arcs.get(i) {
                        eprintln!(
                            "BOOLDBG   across arc {i} {:?} -> {:?} dropped {}",
                            arc.curve.eval(arc.range.0),
                            arc.curve.eval(arc.range.1),
                            dropped.contains(&i)
                        );
                    }
                }
            }
            return Err(Stop::of(IMPRINT_LOST).at(&named));
        }
        regions
    };
    Ok(FaceRegions {
        regions,
        whole,
        apart,
        outline,
    })
}

/// The regions of every face of `own`, classified against `other`, and of
/// those the ones the operation keeps by `table`.
#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    own: &Solid,
    other: &Solid,
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    crossing: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    arcs: &Arcs,
    vertices: &Vertices,
    op: BooleanOp,
    table: Table,
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let other_box = other.bbox();
    let none = BTreeSet::new();
    for face in 0..own.faces() {
        let named = [own.name(face)];
        let f = own.body.faces.get(face).ok_or(Stop::of(LOST))?;
        let coincident = coincident.get(&face).map_or(&[][..], Vec::as_slice);
        let FaceRegions {
            regions,
            whole,
            apart,
            outline,
        } = face_regions(
            own, face, pieces, interior, crossing, coincident, arcs, vertices, &other_box, tol,
        )?;
        for loops in regions {
            // A face that nothing divides is its own outline.
            let made = match outline {
                Some(outline) => {
                    region_outline(outline, whole, &loops, arcs, vertices, tol, &named)?
                }
                None => None,
            };
            let region = made.as_ref().or(outline);
            let class = match (apart, region) {
                (true, _) => Class::Out,
                (false, Some(region)) => {
                    let across = interior.get(&face).unwrap_or(&none);
                    classify(region, &loops, across, other, coincident, tolerance)
                        .map_err(|s| s.at(&named))?
                }
                (false, None) => return Err(Stop::of(LOST).at(&named)),
            };
            let Some(flip) = table(op, own.operand, class).map_err(|r| Stop::of(r).at(&named))?
            else {
                continue;
            };
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
                    operand: own.operand,
                    face: face_id(face),
                },
                surface: f.surface.clone(),
                reversed: f.reversed != flip,
                loops,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::section::prepare;
    use crate::primitives;
    use craft3d_math::Frame;
    use craft3d_topo::Body;

    fn prepared(body: &Body) -> Solid<'_> {
        let tolerance = Tolerance::default();
        let mut vertices = Vertices::new(0.5 * tolerance.linear, 2.0 * tolerance.linear);
        match prepare(body, Operand::A, &mut vertices, &tolerance, false) {
            Ok(solid) => solid,
            Err(_) => panic!("a primitive is a valid operand"),
        }
    }

    fn is_inside(solid: &Solid, p: Vec3) -> bool {
        match inside(solid, p, &Tolerance::default()) {
            Ok(inside) => inside,
            Err(_) => panic!("no ray could tell"),
        }
    }

    /// The first ray tried, as a unit vector.
    fn first_ray() -> Vec3 {
        let (x, y, z) = RAYS[0];
        Vec3::new(x, y, z).normalized().unwrap()
    }

    #[test]
    fn a_ray_through_an_edge_or_a_vertex_is_not_counted() {
        let block = primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
        let solid = prepared(&block);
        let d = first_ray();
        // From inside, straight at the middle of an edge and at a corner:
        // the ray leaves through two or three faces at once.
        for target in [Vec3::new(2.0, 1.0, 2.0), Vec3::new(2.0, 2.0, 2.0)] {
            let p = target - d;
            assert!(block_contains(p));
            assert!(is_inside(&solid, p), "towards {target:?}");
        }
        // From outside, in through a face and out through the edge.
        let p = Vec3::new(2.0, 1.0, 2.0) - d * 3.0;
        assert!(!block_contains(p));
        assert!(!is_inside(&solid, p));
        // And the plain cases.
        assert!(is_inside(&solid, Vec3::new(1.0, 1.0, 1.0)));
        assert!(!is_inside(&solid, Vec3::new(3.0, 1.0, 1.0)));
        assert!(!is_inside(&solid, Vec3::new(1.0, 1.0, -0.5)));
    }

    fn block_contains(p: Vec3) -> bool {
        [p.x, p.y, p.z].iter().all(|c| (0.0..=2.0).contains(c))
    }

    #[test]
    fn a_ray_that_grazes_a_cylinder_is_not_counted() {
        let cylinder = primitives::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap();
        let solid = prepared(&cylinder);
        let d = first_ray();
        // The point of the wall where the first ray is tangent to it.
        let across = Vec3::new(-d.y, d.x, 0.0).normalized().unwrap();
        let touch = across + Vec3::Z;
        let p = touch - d;
        assert!(p.x.hypot(p.y) > 1.0);
        assert!(!is_inside(&solid, p));
        assert!(is_inside(&solid, Vec3::new(0.2, -0.3, 1.0)));
        // Through the rim, where wall and lid meet.
        let rim = Vec3::new(0.6, 0.8, 2.0);
        assert!(is_inside(&solid, rim - d * 0.5));
    }

    #[test]
    fn inside_and_outside_of_cones_spheres_and_tori() {
        let frame = Frame::new(
            Vec3::new(3.0, -1.0, 2.0),
            Vec3::new(0.2, -0.5, 1.0),
            Vec3::new(1.0, 0.3, 0.0),
        )
        .unwrap();
        let cone = primitives::cone(&frame, 2.0, 4.0).unwrap();
        let ball = primitives::sphere(&frame, 2.0).unwrap();
        let ring = primitives::torus(&frame, 4.0, 1.0).unwrap();
        let (cone, ball, ring) = (prepared(&cone), prepared(&ball), prepared(&ring));
        let mut seed = 11_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut counts = [0_usize; 3];
        for _ in 0..600 {
            let (x, y, z) = (unit() * 12.0, unit() * 12.0, unit() * 10.0);
            let p = frame.point(x, y, z);
            let rho = x.hypot(y);
            // Well away from the boundary, where rounding cannot decide.
            let in_cone = (rho - (2.0 - 0.5 * z), -z);
            let in_ball = Vec3::new(x, y, z).length() - 2.0;
            let in_ring = (rho - 4.0).hypot(z) - 1.0;
            if in_cone.0.abs() > 1e-3 && in_cone.1.abs() > 1e-3 {
                let expected = in_cone.0 < 0.0 && in_cone.1 < 0.0;
                assert_eq!(is_inside(&cone, p), expected, "cone at ({x}, {y}, {z})");
                counts[0] += usize::from(expected);
            }
            if in_ball.abs() > 1e-3 {
                assert_eq!(
                    is_inside(&ball, p),
                    in_ball < 0.0,
                    "ball at ({x}, {y}, {z})"
                );
                counts[1] += usize::from(in_ball < 0.0);
            }
            if in_ring.abs() > 1e-3 {
                assert_eq!(
                    is_inside(&ring, p),
                    in_ring < 0.0,
                    "ring at ({x}, {y}, {z})"
                );
                counts[2] += usize::from(in_ring < 0.0);
            }
        }
        // Some of the points were inside each.
        assert!(counts.iter().all(|&n| n > 3), "{counts:?}");
        // From the middle of the ring's hole every ray crosses the tube
        // twice or not at all.
        assert!(!is_inside(&ring, frame.origin()));
    }
}
