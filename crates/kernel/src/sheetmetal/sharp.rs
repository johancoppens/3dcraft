//! Sharp bends: a bend of inner radius 0 (docs/design/sheetmetal.md ›
//! *Sharp bends*).
//!
//! The model keeps a bend's skins at `R` and `R + T` about its axis, so at
//! `R = 0` the inner skin is the axis itself (a line where the two flats'
//! inner skins meet) and the outer skin a cylinder of radius `T`: the bend's
//! material is the sector of the disc of radius `T` over the angle, its
//! volume `θ w T²/2`, the bend allowance `θ K T` and the deduction
//! `2 T tan(θ/2) − θ K T`, the closed forms of every other bend at `R = 0`.
//!
//! The plate of the bottom skin cannot make it (the offset of the cylinder
//! of radius `T` by `T` towards its axis is a line, and a bend down's
//! bottom skin would be one), so a sharp bend has no face of the folded
//! sheet: its two tangent lines end the flats' sheets. The pieces the sheet
//! falls into are plated apart, each with its square rims, and every sharp
//! bend is **joined** in exactly: the two rims along its tangent lines (each
//! the rectangle from the axis to the outer skin, which the bend's sector
//! has as its two flat sides) are taken out, the outer skin's cylinder and
//! the two end sectors are put in, sharing the plates' own edges and
//! vertices, and the child's inner edge goes into the parent's (both lie on
//! the axis). Nothing is moved and no boolean is run; the result is checked
//! as every body is.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{
    Bend, Corner, CornerKind, EdgeKey, Flat, P2, SLIVER_TOLERANCES, lift, outline_key, refuse,
    seat_of, unit, world,
};
use crate::KernelError;
use crate::plate::{Joint, Made, Plate, Side};
use crate::profile::{cross, distance, dot, sub};
use craft3d_geom::{Circle, Curve3, Cylinder, Plane, Surface};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// An inner radius within the tolerance of 0 is 0: the bend is sharp.
pub(super) fn snapped(radius: f64, tol: f64) -> f64 {
    if radius <= tol { 0.0 } else { radius }
}

/// What a sharp bend refuses on its own: 180 degrees (the flange folded
/// onto the parent's inner skin, the two touching all over: a closed hem,
/// which one solid cannot hold), and an allowance `θ K T` within
/// [`SLIVER_TOLERANCES`] tolerances of 0 (at `K = 0` the strip has no
/// width and the two tangent lines would be one line in the pattern). The
/// outer skin's arcs, `θ T` long, are then longer still (`K ≤ 1`).
pub(super) fn check(
    i: usize,
    angle: f64,
    k: f64,
    t: f64,
    tol: f64,
    angular: f64,
) -> Result<(), KernelError> {
    if !(angle < PI - angular) {
        return Err(refuse(
            Some(i),
            "a sharp bend (inner radius 0) of 180 degrees folds the flange onto the parent's inner skin, touching it all over (a closed hem), which one solid cannot hold; give the bend the radius the hem is pressed to, or an angle below 180 degrees",
        ));
    }
    let allowance = angle * k * t;
    if !(allowance > SLIVER_TOLERANCES * tol) {
        return Err(refuse(
            Some(i),
            format!(
                "a sharp bend's allowance θ K T is {allowance}, within {SLIVER_TOLERANCES} tolerances of 0 (at K = 0 the neutral fibre is the inner edge, of no length, and the strip would have no width in the flat pattern); give a K-factor above 0"
            ),
        ));
    }
    Ok(())
}

/// What the parts with a sharp bend do not build, each refused with the
/// reason (docs/design/sheetmetal.md › *Sharp bends*).
///
/// At `R = 0` a wall's inner skin is the plane through its tangent line,
/// and the parent's inner skin runs into the same line, so material that a
/// radius kept `R` away from either now touches it; the checker does not
/// see two pieces that only touch (review of d88ec622). What can touch
/// that way is refused, each by a rule that holds for any part:
///
/// - **Two bends that meet** (an end of one tangent line on an end of the
///   other's, on one parent, turning the same way) where one is sharp and
///   the other sharp too: each wall's inner skin runs through the meeting
///   point, so the walls share a piece of a line there. Unless their
///   corner is closed (the flats end on planes a gap apart) or relieved
///   (then the tangent lines do not meet).
/// - **Anything else on either inner skin**: [`clear`], a proof for every
///   piece of the part (review of 8449951d: a rule per case missed a lip
///   on a neighbouring wall's tip, and decided "square" by an angle where
///   contact is a length against the tolerance).
/// - **A flange on an edge of a flat that starts at a sharp bend's far
///   tangent line, or a sharp one on an edge that starts at its parent's
///   bend's**: the edge's end lies on the parent's inner skin (sharp) or
///   the sharp flange's inner skin through it, so the flange's rim along
///   it lies on the other's skin (a tab on a side of a sharp wall).
/// - **Folded flat**: the far end of a sharp bend's flat no more than
///   [`SLIVER_TOLERANCES`] tolerances from the parent's inner skin
///   (`L sin θ`), touching it or a sliver from it.
/// - A trimmed sharp bend (owed). A hole across a sharp bend is decided
///   where the holes are cut into the pattern (`cuts.rs`).
pub(super) fn refused(bends: &[Bend], corners: &[Corner], tol: f64) -> Result<(), KernelError> {
    for (i, b) in bends.iter().enumerate() {
        if !b.sharp() {
            continue;
        }
        if b.trim.iter().any(Option::is_some) {
            return Err(refuse(
                Some(i),
                "its bend is sharp (inner radius 0) and a closed corner's end plane cuts it: trimming a sharp bend is not built (owed); give the corner a relief, or the bend a radius",
            ));
        }
        let gap = b.flat_length * b.theta().psin();
        if !(gap > SLIVER_TOLERANCES * tol) {
            return Err(refuse(
                Some(i),
                format!(
                    "a sharp bend (inner radius 0) folds its flat to {gap} from the parent's inner skin (its length times the sine of the angle), within {SLIVER_TOLERANCES} tolerances: touching it or a sliver from it (a closed hem), which one solid cannot hold; give the bend the radius the hem is pressed to, or a smaller angle"
                ),
            ));
        }
    }
    let closed = |x: usize, y: usize| {
        corners
            .iter()
            .any(|c| c.closing.kind != CornerKind::Open && (c.bends == [x, y] || c.bends == [y, x]))
    };
    let ends = |b: &Bend| [b.start, b.end];
    let on_segment = |p: P2, a: P2, e: P2| {
        let Some(d) = unit(sub(e, a)) else {
            return false;
        };
        let s = dot(sub(p, a), d);
        let off = cross(d, sub(p, a)).abs();
        off <= tol && s >= -tol && s <= distance(a, e) + tol
    };
    for (i, s) in bends.iter().enumerate() {
        if !s.sharp() {
            continue;
        }
        for (j, o) in bends.iter().enumerate() {
            if j == i || o.parent != s.parent || (o.angle > 0.0) != (s.angle > 0.0) {
                continue;
            }
            let meet = ends(s)
                .iter()
                .any(|p| ends(o).iter().any(|q| distance(*p, *q) <= tol));
            if meet && o.sharp() && !closed(i, j) {
                return Err(refuse(
                    Some(i.max(j)),
                    format!(
                        "it and flange {} meet with sharp bends (inner radius 0) turning the same way, open and without a relief: each wall's inner skin runs through the point where their bends meet, so the two walls would touch along a line there, which one solid cannot hold; give the part a relief, close the corner, or give a bend a radius",
                        i.min(j)
                    ),
                ));
            }
        }
    }
    // A flange on an edge of a flat that starts where the flat's own bend
    // ends, when either bend is sharp.
    for (j, o) in bends.iter().enumerate() {
        let Some((w, wb)) = bends.iter().enumerate().find(|(_, w)| w.child == o.parent) else {
            continue;
        };
        if !(o.sharp() || wb.sharp()) {
            continue;
        }
        let (fa, fe) = wb.far_line();
        if ends(o).iter().any(|p| on_segment(*p, fa, fe)) {
            return Err(refuse(
                Some(j),
                format!(
                    "it sits on an edge of flange {w}'s flat that starts at that flange's bend (a side), and one of the two bends is sharp (inner radius 0): the edge's end lies on the other's inner skin, so this flange's rim would touch it, which one solid cannot hold; give the bends a radius, or inset this flange from the bend"
                ),
            ));
        }
    }
    Ok(())
}

/// A piece of the folded part: a flat or a bend (by its index), or a box
/// in a flat's axes that holds a formed louvre's hood (added by booleans
/// after the part is built, so no flat or bend of it).
#[derive(Clone, Copy)]
pub(super) enum Piece {
    Flat(usize),
    Bend(usize),
    /// The louvre's place, its flat, the box's `[x0, y0, x1, y1]` in the
    /// pattern and its `z` range in the flat's axes.
    Hood(usize, usize, [f64; 4], (f64, f64)),
}

impl Piece {
    /// The least and the most of `f` over the piece's material (lower and
    /// upper bounds: `flat_least`, `bend_least`, a box's corners).
    pub(super) fn range(
        self,
        flats: &[Flat],
        bends: &[Bend],
        t: f64,
        f: impl Fn(Vec3) -> f64,
    ) -> Option<(f64, f64)> {
        match self {
            Piece::Flat(i) => {
                let flat = flats.get(i)?;
                Some((
                    super::flat_least(flat, &f, t)?,
                    -super::flat_least(flat, |q| -f(q), t)?,
                ))
            }
            Piece::Bend(i) => {
                let x = bends.get(i)?;
                let tr = flats.get(x.parent)?.transform;
                Some((
                    super::bend_least(x, &tr, &f)?,
                    -super::bend_least(x, &tr, |q| -f(q))?,
                ))
            }
            Piece::Hood(_, flat, [x0, y0, x1, y1], (z0, z1)) => {
                let tr = flats.get(flat)?.transform;
                let mut out = (f64::INFINITY, f64::NEG_INFINITY);
                for x in [x0, x1] {
                    for y in [y0, y1] {
                        for z in [z0, z1] {
                            let v = f(tr.point(Vec3::new(x, y, z)));
                            out = (out.0.min(v), out.1.max(v));
                        }
                    }
                }
                (out.0.is_finite() && out.1.is_finite()).then_some(out)
            }
        }
    }

    /// The piece in the words the part's errors use (`flanges` the
    /// flanges' count: the bends past it are jogs', two each, and their
    /// flats a web and a far side).
    pub(super) fn named(self, flanges: usize) -> String {
        let jog = |k: usize| ((k - flanges) / 2, (k - flanges) % 2);
        match self {
            Piece::Flat(0) => "the base".to_string(),
            Piece::Flat(f) if f - 1 < flanges => format!("flange {}'s flat", f - 1),
            Piece::Flat(f) => match jog(f - 1) {
                (g, 0) => format!("jog {g}'s web"),
                (g, _) => format!("jog {g}'s far side"),
            },
            Piece::Bend(b) if b < flanges => format!("flange {b}'s bend"),
            Piece::Bend(b) => match jog(b) {
                (g, 0) => format!("jog {g}'s first bend"),
                (g, _) => format!("jog {g}'s second bend"),
            },
            Piece::Hood(g, ..) => format!("formed louvre {g}'s hood"),
        }
    }
}

/// A piece of a flat's inner skin by a sharp bend, in the folded part:
/// the rectangle `o + u e1 + v e2` over the ranges given, `n` its normal
/// away from the flat's material.
struct Skin {
    o: Vec3,
    n: Vec3,
    e1: Vec3,
    e2: Vec3,
    u: (f64, f64),
    v: (f64, f64),
}

impl Skin {
    /// Whether the piece is proven more than `tol` from the skin's
    /// rectangle taken `inset` in from its edges: wholly past one of its
    /// six bounding planes by `tol` (a separating axis; failing that, not
    /// proven).
    fn apart(
        &self,
        piece: Piece,
        flats: &[Flat],
        bends: &[Bend],
        t: f64,
        tol: f64,
        inset: f64,
    ) -> Option<bool> {
        let (u, v) = (
            (self.u.0 + inset, self.u.1 - inset),
            (self.v.0 + inset, self.v.1 - inset),
        );
        self.apart_in(piece, flats, bends, t, tol, u, v)
    }

    /// [`Skin::apart`] from the rectangle over `u` × `v`.
    #[allow(clippy::too_many_arguments)]
    fn apart_in(
        &self,
        piece: Piece,
        flats: &[Flat],
        bends: &[Bend],
        t: f64,
        tol: f64,
        u: (f64, f64),
        v: (f64, f64),
    ) -> Option<bool> {
        self.apart_along(piece, flats, bends, t, tol, u, v, &[])
    }

    /// [`Skin::apart_in`] trying, besides the rectangle's own normals, the
    /// directions `c1 e1 + c2 e2` given (unit or not: each is made unit).
    #[allow(clippy::too_many_arguments)]
    fn apart_along(
        &self,
        piece: Piece,
        flats: &[Flat],
        bends: &[Bend],
        t: f64,
        tol: f64,
        u: (f64, f64),
        v: (f64, f64),
        more: &[(f64, f64)],
    ) -> Option<bool> {
        let o = self.o;
        if u.0 > u.1 || v.0 > v.1 {
            return Some(true);
        }
        for (axis, (lo, hi)) in [(self.n, (0.0, 0.0)), (self.e1, u), (self.e2, v)] {
            let (a, b) = piece.range(flats, bends, t, |q| axis.dot(q - o))?;
            if b < lo - tol || a > hi + tol {
                return Some(true);
            }
        }
        for &(c1, c2) in more {
            let norm = c1.phypot(c2);
            if !(norm > 0.0) {
                continue;
            }
            let (c1, c2) = (c1 / norm, c2 / norm);
            let axis = self.e1 * c1 + self.e2 * c2;
            // The rectangle's range along it: at its corners.
            let at = |x: f64, y: f64| c1 * x + c2 * y;
            let corners = [at(u.0, v.0), at(u.0, v.1), at(u.1, v.0), at(u.1, v.1)];
            let lo = corners.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let (a, b) = piece.range(flats, bends, t, |q| axis.dot(q - o))?;
            if b < lo - tol || a > hi + tol {
                return Some(true);
            }
        }
        Some(false)
    }
}

/// Every sharp bend's two inner skins kept clear (docs/design/sheetmetal.md
/// › *Sharp bends* › *Touching*). At `R = 0` the child's inner skin is the
/// plane through the bend's axis and the parent's runs into the same line,
/// so the bend's radius no longer keeps the child's side of the part
/// (the child's flat and everything it carries) `R` from the rest: what
/// the radius alone kept apart may now touch, which the checker does not
/// see. So for each sharp bend, the child's inner skin (the rectangle of
/// its flat's outline along and across the axis, a superset of the face)
/// must be proven more than the tolerance from every piece on the other
/// side (all but the child's subtree, the parent and the bend), and the
/// parent's inner skin from every piece of the child's side but the child
/// flat. Proven against the whole rectangle; a piece proven apart only
/// from the rectangle taken [`SLIVER_TOLERANCES`] tolerances in from its
/// edges (it reaches the skin only along an edge) is accepted only when it
/// is joined there: a bend of the same parent with an end on this bend's
/// tangent line's end (the neighbour at a corner, which meets the skin
/// along the corner's line below the axis). Anything else not proven
/// apart refuses the part, naming both.
///
/// The neighbour is proven apart from the whole skin but the square
/// [`SLIVER_TOLERANCES`] tolerances on a side at the corner it shares
/// (re-verification of d8ecd192): from the skin less a strip that wide
/// along the shared end, and from the skin less a strip that wide along
/// the axis.
pub(super) fn clear(
    flats: &[Flat],
    bends: &[Bend],
    flanges: usize,
    hoods: &[Piece],
    t: f64,
    tol: f64,
) -> Result<(), KernelError> {
    let m = SLIVER_TOLERANCES * tol;
    for (i, b) in bends.iter().enumerate() {
        if !b.sharp() {
            continue;
        }
        let up = b.angle > 0.0;
        let z_in = if up { t } else { 0.0 };
        let sign = if up { 1.0 } else { -1.0 };
        let d = unit(sub(b.end, b.start)).ok_or_else(|| refuse(Some(i), "a bend has no length"))?;
        // The child's side: its flat and every flat and bend it carries.
        let mut side: Vec<usize> = vec![b.child];
        let mut carried: Vec<usize> = Vec::new();
        for _ in 0..=bends.len() {
            let mut grew = false;
            for (j, x) in bends.iter().enumerate() {
                if side.contains(&x.parent) && !carried.contains(&j) {
                    carried.push(j);
                    if !side.contains(&x.child) {
                        side.push(x.child);
                    }
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let skin_of = |flat: usize, origin: P2, along: P2, across: P2| -> Option<Skin> {
            let f = flats.get(flat)?;
            let tr = f.transform;
            let (mut u, mut v) = (
                (f64::INFINITY, f64::NEG_INFINITY),
                (f64::INFINITY, f64::NEG_INFINITY),
            );
            for seg in &f.outer {
                let (a, c) = super::reach_along(seg, origin, along);
                u = (u.0.min(a), u.1.max(c));
                let (a, c) = super::reach_along(seg, origin, across);
                v = (v.0.min(a), v.1.max(c));
            }
            Some(Skin {
                o: tr.point(Vec3::new(origin[0], origin[1], z_in)),
                n: tr.vector(Vec3::Z) * sign,
                e1: tr.vector(lift(along)),
                e2: tr.vector(lift(across)),
                u,
                v: (v.0.max(0.0), v.1),
            })
        };
        let lost = || refuse(Some(i), "a sharp bend's flats are missing");
        let (far, _) = b.far_line();
        let child = skin_of(b.child, far, d, b.across).ok_or_else(lost)?;
        let parent =
            skin_of(b.parent, b.start, d, [-b.across[0], -b.across[1]]).ok_or_else(lost)?;
        // The end of this bend's tangent line (0 its start, 1 its end) a
        // bend of the same parent has an end on.
        let joined_at = |j: usize| -> Option<usize> {
            let x = bends.get(j)?;
            if x.parent != b.parent {
                return None;
            }
            [b.start, b.end]
                .iter()
                .position(|q| [x.start, x.end].iter().any(|p| distance(*p, *q) <= tol))
        };
        let pieces: Vec<Piece> = if hoods.is_empty() {
            (0..flats.len())
                .map(Piece::Flat)
                .chain((0..bends.len()).map(Piece::Bend))
                .collect()
        } else {
            hoods.to_vec()
        };
        for piece in pieces {
            let ours = match piece {
                Piece::Flat(f) | Piece::Hood(_, f, ..) => side.contains(&f),
                Piece::Bend(j) => carried.contains(&j),
            };
            // The parent and the child meet the other's skin only along
            // the axis, at the bend's angle (folded onto it: the near-hem
            // rule); the bend joins them.
            let skip = match piece {
                Piece::Flat(f) => f == b.parent || f == b.child,
                Piece::Bend(j) => j == i,
                Piece::Hood(..) => false,
            };
            if skip {
                continue;
            }
            let (skin, whose) = if ours {
                (&parent, "its parent's inner skin")
            } else {
                (&child, "its inner skin")
            };
            let whole = skin
                .apart(piece, flats, bends, t, tol, 0.0)
                .ok_or_else(lost)?;
            // A neighbour joined at a corner: apart from the whole skin but
            // the m × m square at that corner (the union of the skin less a
            // strip m wide along the shared end and the skin less a strip m
            // wide along the axis), where it meets the skin along the
            // corner's line below the axis.
            let along_edge = || -> Result<bool, KernelError> {
                let Piece::Bend(j) = piece else {
                    return Ok(false);
                };
                let Some(end) = joined_at(j).filter(|_| !ours) else {
                    return Ok(false);
                };
                let (u, v) = (skin.u, skin.v);
                let beside = if end == 0 {
                    (u.0 + m, u.1)
                } else {
                    (u.0, u.1 - m)
                };
                let off_end = skin
                    .apart_in(piece, flats, bends, t, tol, beside, v)
                    .ok_or_else(lost)?;
                // Off the axis, the neighbour leaves the corner's line
                // outward and upward: the directions out of the shared end
                // tilted down the skin separate it (one is enough).
                let out = if end == 0 { -1.0 } else { 1.0 };
                let tilted = [(out, -0.5), (out, -1.0), (out, -2.0), (out, -4.0)];
                let off_axis = skin
                    .apart_along(piece, flats, bends, t, tol, u, (v.0 + m, v.1), &tilted)
                    .ok_or_else(lost)?;
                Ok(off_end && off_axis)
            };
            if !whole && !along_edge()? {
                let named = piece.named(flanges);
                let reason = format!(
                    "flange {i}'s bend is sharp (inner radius 0), and {named} is not proven clear of {whose} (the plane through that bend's tangent line, where a radius would keep material away): touching it, or within the tolerance, which one solid cannot hold; give the bend a radius, or move what comes near it"
                );
                return Err(match piece {
                    Piece::Hood(g, ..) => refuse(Some(g), reason),
                    _ => refuse(Some(i), reason),
                });
            }
        }
    }
    Ok(())
}

/// [`clear`] for the hoods of formed louvres, each bounded by the box of
/// its footprint grown by the thickness, from its flat's skin to `h + T`
/// beyond it (the hood's skins lie within: the section rises to `h`, its
/// other skin `T` further along its normal).
pub(super) fn clear_hoods(
    part: &super::Part,
    formed: &[super::formed::Formed],
    tol: f64,
) -> Result<(), KernelError> {
    if !part.bends.iter().any(Bend::sharp) {
        return Ok(());
    }
    let t = part.thickness;
    let mut hoods = Vec::with_capacity(formed.len());
    for (g, f) in formed.iter().enumerate() {
        let (mut x, mut y) = (
            (f64::INFINITY, f64::NEG_INFINITY),
            (f64::INFINITY, f64::NEG_INFINITY),
        );
        for seg in &f.footprint.outer.segments {
            let (a, c) = super::reach_along(seg, [0.0, 0.0], [1.0, 0.0]);
            x = (x.0.min(a), x.1.max(c));
            let (a, c) = super::reach_along(seg, [0.0, 0.0], [0.0, 1.0]);
            y = (y.0.min(a), y.1.max(c));
        }
        let h = f.spec.height;
        let z = if f.spec.up { (0.0, h + t) } else { (-h, t) };
        hoods.push(Piece::Hood(
            g,
            f.flat,
            [x.0 - t, y.0 - t, x.1 + t, y.1 + t],
            z,
        ));
    }
    if hoods.is_empty() {
        return Ok(());
    }
    clear(&part.flats, &part.bends, part.flanges, &hoods, t, tol)
}

/// A face joined to the plates at a sharp bend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharpFace {
    /// The outer skin: the cylinder of radius `T` about the bend's axis
    /// (the bottom skin for a bend up, the top for a bend down); with
    /// holes across the bend, its first stretch along the axis.
    Skin,
    /// The sector at the bend's start (0) or end (1).
    End(usize),
    /// The outer skin's further stretch `j` (1, 2, ...) along the axis
    /// that holes across the bend leave it in.
    SkinPiece(usize),
    /// The sector at the side of a hole across the bend that ends
    /// stretch `j`: the rim along that hole's piece across the strip
    /// (`EdgeKey::Cut`).
    Side(usize, EdgeKey),
}

/// A hole's piece straight across a sharp bend's strip: the bend, where
/// along its tangent line it lies, and what the piece is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SharpSide {
    pub bend: usize,
    pub u: f64,
    pub key: EdgeKey,
}

/// The most steps an alias is followed (each joins one edge or vertex
/// into another made before it, so a chain is at most as long as the
/// sharp bends; the bound only keeps a bad chain from running on).
const MAX_ALIAS: usize = 1 << 16;

/// The plates of the folded sheet's pieces, with every sharp bend joined
/// in (see the module documentation).
#[allow(clippy::too_many_arguments)]
pub(super) fn plate(
    sheet: &Body,
    sides: &[SharpSide],
    edge_keys: &[EdgeKey],
    flats: &[Flat],
    bends: &[Bend],
    plane: &Frame,
    t: f64,
    joints: &BTreeMap<EdgeId, Joint<'_>>,
    tolerance: &Tolerance,
) -> Result<(Plate, Vec<(usize, SharpFace)>), KernelError> {
    let tol = tolerance.linear;
    // Each piece plated apart, its joints those on its own edges.
    let mut body = Body::new(BodyKind::Solid);
    let mut made: Vec<Made> = Vec::new();
    for faces in &pieces(sheet) {
        let edges: BTreeSet<u32> = faces
            .iter()
            .filter_map(|f| sheet.face(*f))
            .flat_map(|f| f.loops.iter())
            .filter_map(|l| sheet.loop_(*l))
            .flat_map(|l| l.coedges.iter().map(|c| c.edge.0))
            .collect();
        let own: BTreeMap<EdgeId, Joint<'_>> = joints
            .iter()
            .filter(|(e, _)| edges.contains(&e.0))
            .map(|(e, j)| (*e, *j))
            .collect();
        let p = crate::plate::plate(sheet, faces, t, Side::Out, &own, tolerance)?;
        append(&mut body, &mut made, &p)?;
    }
    let mut join = Join {
        body,
        made,
        joined: Vec::new(),
        dropped: BTreeSet::new(),
        vertex_alias: BTreeMap::new(),
        edge_alias: BTreeMap::new(),
    };
    for (i, bend) in bends.iter().enumerate() {
        if bend.sharp() {
            join.bend(i, bend, sheet, edge_keys, sides, flats, plane, t, tol)?;
        }
    }
    let joined = std::mem::take(&mut join.joined);
    let solid = join.finish()?;
    let mut defects = craft3d_topo::check_remembered(&solid.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sheet metal (sharp bends)",
            defects,
        });
    }
    Ok((solid, joined))
}

/// The pieces of a sheet: its faces that hang together across shared
/// edges, each in face order, the pieces in the order of their first face.
pub(super) fn pieces(sheet: &Body) -> Vec<Vec<FaceId>> {
    let mut users: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (f, face) in sheet.faces.iter().enumerate() {
        for l in &face.loops {
            for c in sheet.loop_(*l).map_or(&[][..], |l| l.coedges.as_slice()) {
                users.entry(c.edge.0).or_default().push(f);
            }
        }
    }
    let mut group: Vec<usize> = (0..sheet.faces.len()).collect();
    // Each link points to a lower face, so a walk ends within the faces.
    let root = |group: &[usize], mut i: usize| -> usize {
        for _ in 0..group.len() {
            match group.get(i) {
                Some(&up) if up != i => i = up,
                _ => break,
            }
        }
        i
    };
    for list in users.values() {
        if let [a, b] = list.as_slice() {
            let (ra, rb) = (root(&group, *a), root(&group, *b));
            if let Some(slot) = group.get_mut(ra.max(rb)) {
                *slot = ra.min(rb);
            }
        }
    }
    let mut pieces: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for f in 0..sheet.faces.len() {
        if let Ok(id) = u32::try_from(f) {
            pieces.entry(root(&group, f)).or_default().push(FaceId(id));
        }
    }
    pieces.into_values().collect()
}

fn id32(n: usize) -> Result<u32, KernelError> {
    u32::try_from(n).map_err(|_| refuse(None, "too many entities"))
}

/// Puts plate `p`'s body after what `body` has, its faces' origins after
/// `made`'s. The plates' tables of loose tolerances are not carried: a
/// plate the kernel builds records none (`Body::tolerances` is empty for
/// every body it makes), so there is nothing to carry.
fn append(body: &mut Body, made: &mut Vec<Made>, p: &Plate) -> Result<(), KernelError> {
    let (v0, e0, l0) = (
        id32(body.vertices.len())?,
        id32(body.edges.len())?,
        id32(body.loops.len())?,
    );
    let shift = |a: u32, by: u32| {
        a.checked_add(by)
            .ok_or_else(|| refuse(None, "too many entities"))
    };
    body.vertices.extend(p.body.vertices.iter().cloned());
    for e in &p.body.edges {
        let mut e = e.clone();
        e.start = VertexId(shift(e.start.0, v0)?);
        e.end = VertexId(shift(e.end.0, v0)?);
        body.edges.push(e);
    }
    for l in &p.body.loops {
        let coedges = l
            .coedges
            .iter()
            .map(|c| {
                Ok(Coedge {
                    edge: EdgeId(shift(c.edge.0, e0)?),
                    reversed: c.reversed,
                })
            })
            .collect::<Result<Vec<_>, KernelError>>()?;
        body.loops.push(craft3d_topo::Loop { coedges });
    }
    for f in &p.body.faces {
        let mut f = f.clone();
        for l in &mut f.loops {
            *l = craft3d_topo::LoopId(shift(l.0, l0)?);
        }
        body.faces.push(f);
    }
    made.extend(p.faces.iter().copied());
    Ok(())
}

/// The plates put together, with what the joining takes out and joins.
struct Join {
    body: Body,
    made: Vec<Made>,
    /// What each [`Made::Joined`] number stands for: the bend and the
    /// face (the number is the place in this list).
    joined: Vec<(usize, SharpFace)>,
    /// The rims taken out.
    dropped: BTreeSet<usize>,
    /// A vertex that went into another.
    vertex_alias: BTreeMap<u32, u32>,
    /// An edge that went into another, and whether it ran against it.
    edge_alias: BTreeMap<u32, (u32, bool)>,
}

/// A rim along a tangent line: its four edges as it uses them.
struct Quad {
    inner: Coedge,
    outer: Coedge,
    sides: [Coedge; 2],
}

impl Join {
    /// Records the face just added as `face` of sharp bend `bend`.
    fn joined_face(&mut self, bend: usize, face: SharpFace) -> Result<(), KernelError> {
        let code = u32::try_from(self.joined.len())
            .map_err(|_| refuse(Some(bend), "too many faces to number"))?;
        self.joined.push((bend, face));
        self.made.push(Made::Joined(code));
        Ok(())
    }

    fn vertex(&self, v: VertexId) -> VertexId {
        let mut v = v.0;
        for _ in 0..MAX_ALIAS {
            match self.vertex_alias.get(&v) {
                Some(&to) => v = to,
                None => break,
            }
        }
        VertexId(v)
    }

    fn coedge(&self, c: Coedge) -> Coedge {
        let (mut e, mut rev) = (c.edge.0, c.reversed);
        for _ in 0..MAX_ALIAS {
            match self.edge_alias.get(&e) {
                Some(&(to, flip)) => {
                    e = to;
                    rev ^= flip;
                }
                None => break,
            }
        }
        Coedge {
            edge: EdgeId(e),
            reversed: rev,
        }
    }

    /// Where a coedge runs from and to, through the aliases.
    fn ends(&self, c: Coedge) -> Option<(VertexId, VertexId)> {
        let c = self.coedge(c);
        let (a, b) = self.body.coedge_ends(c)?;
        Some((self.vertex(a), self.vertex(b)))
    }

    fn point(&self, v: VertexId) -> Option<Vec3> {
        self.body.vertex(self.vertex(v)).map(|v| v.point)
    }

    /// Per edge (through the aliases), the faces left that use it.
    fn users(&self) -> BTreeMap<u32, Vec<usize>> {
        let mut users: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (f, face) in self.body.faces.iter().enumerate() {
            if self.dropped.contains(&f) {
                continue;
            }
            for l in &face.loops {
                for c in self
                    .body
                    .loop_(*l)
                    .map_or(&[][..], |l| l.coedges.as_slice())
                {
                    users.entry(self.coedge(*c).edge.0).or_default().push(f);
                }
            }
        }
        users
    }

    /// The rim along the sheet's edge `edge`, as a quad: its inner and
    /// outer edges (shared with the flat's skins) and its two sides.
    fn quad(
        &self,
        i: usize,
        edge: usize,
        flat_face: FaceId,
        up: bool,
    ) -> Result<(usize, Quad), KernelError> {
        let want = Made::Rim(EdgeId(id32(edge)?));
        let rims: Vec<usize> = (0..self.made.len())
            .filter(|f| self.made.get(*f) == Some(&want) && !self.dropped.contains(f))
            .collect();
        let [rim] = rims.as_slice() else {
            return Err(refuse(
                Some(i),
                "its bend is sharp, and the plate made no one rim along its tangent line",
            ));
        };
        let face = self
            .body
            .faces
            .get(*rim)
            .ok_or_else(|| refuse(Some(i), "a rim is missing"))?;
        let coedges: Vec<Coedge> = match face.loops.as_slice() {
            [l] => self
                .body
                .loop_(*l)
                .map(|l| l.coedges.iter().map(|c| self.coedge(*c)).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if coedges.len() != 4 {
            return Err(refuse(
                Some(i),
                "its bend is sharp, and the rim along its tangent line is not a rectangle (a hole or a cut on the tangent line): not built (owed)",
            ));
        }
        let users = self.users();
        let (mut top, mut bottom, mut sides) = (None, None, Vec::new());
        for c in coedges {
            let others: Vec<usize> = users
                .get(&c.edge.0)
                .map(|l| l.iter().copied().filter(|f| f != rim).collect())
                .unwrap_or_default();
            let [other] = others.as_slice() else {
                return Err(refuse(
                    Some(i),
                    "an edge of the rim along a sharp bend's tangent line is not shared by one other face",
                ));
            };
            match self.made.get(*other) {
                Some(Made::Front(f)) if *f == flat_face => top = Some(c),
                Some(Made::Back(f)) if *f == flat_face => bottom = Some(c),
                _ => sides.push(c),
            }
        }
        let (Some(top), Some(bottom), [s0, s1]) = (top, bottom, sides.as_slice()) else {
            return Err(refuse(
                Some(i),
                "the rim along a sharp bend's tangent line does not run between the flat's two skins",
            ));
        };
        let (inner, outer) = if up { (top, bottom) } else { (bottom, top) };
        Ok((
            *rim,
            Quad {
                inner,
                outer,
                sides: [*s0, *s1],
            },
        ))
    }

    /// Of a coedge's two vertices, the one nearer `p`, if within `tol`.
    fn nearest(&self, c: Coedge, p: Vec3, tol: f64) -> Option<VertexId> {
        let (a, b) = self.ends(c)?;
        let (da, db) = (self.point(a)?.distance(p), self.point(b)?.distance(p));
        let (v, d) = if da <= db { (a, da) } else { (b, db) };
        (d <= tol).then_some(v)
    }

    /// Joins sharp bend `i` in: per stretch of it that no hole across it
    /// takes away (the whole bend without one), the outer skin and the two
    /// end sectors between the rims along its two tangent lines' pieces
    /// over that stretch ([`Join::stretch`]).
    #[allow(clippy::too_many_arguments)]
    fn bend(
        &mut self,
        i: usize,
        bend: &Bend,
        sheet: &Body,
        edge_keys: &[EdgeKey],
        sides: &[SharpSide],
        flats: &[Flat],
        plane: &Frame,
        t: f64,
        tol: f64,
    ) -> Result<(), KernelError> {
        let seat = seat_of(flats, i, bend)?;
        let parent_key = match seat.hole {
            None => outline_key(flats, bend.parent, seat.seg),
            Some(hole) => EdgeKey::Hole {
                flat: bend.parent,
                hole,
                seg: seat.seg,
            },
        };
        let child_key = outline_key(flats, bend.child, 0);
        // The tangent lines' edges: whole, or the pieces a hole across the
        // bend leaves of them.
        let on_parent = |k: &EdgeKey| match (*k, seat.hole) {
            (EdgeKey::Split { flat, seg, .. }, None) => flat == bend.parent && seg == seat.seg,
            (
                EdgeKey::SplitHole {
                    flat, hole, seg, ..
                },
                Some(h),
            ) => flat == bend.parent && hole == h && seg == seat.seg,
            _ => *k == parent_key,
        };
        let on_child = |k: &EdgeKey| match *k {
            EdgeKey::Split { flat, seg: 0, .. } => flat == bend.child,
            _ => *k == child_key,
        };
        let d = unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?;
        let wp = world(
            plane,
            flats
                .get(bend.parent)
                .ok_or_else(|| refuse(Some(i), "a bend's parent is missing"))?,
        );
        let axis = wp
            .vector(lift(d))
            .normalized()
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let origin = wp.point(lift(bend.start));
        // Per edge, its stretch along the axis.
        let stretch = |e: usize| -> Option<(f64, f64, usize)> {
            let edge = sheet.edge(EdgeId(u32::try_from(e).ok()?))?;
            let a = (sheet.vertex(edge.start)?.point - origin).dot(axis);
            let b = (sheet.vertex(edge.end)?.point - origin).dot(axis);
            Some((a.min(b), a.max(b), e))
        };
        let lost = || refuse(Some(i), "a sharp bend's tangent line has no edge");
        let mut parents = Vec::new();
        let mut children = Vec::new();
        for (e, k) in edge_keys.iter().enumerate() {
            if on_parent(k) {
                parents.push(stretch(e).ok_or_else(lost)?);
            } else if on_child(k) {
                children.push(stretch(e).ok_or_else(lost)?);
            }
        }
        parents.sort_by(|a, b| a.0.total_cmp(&b.0));
        children.sort_by(|a, b| a.0.total_cmp(&b.0));
        let same = parents.len() == children.len()
            && !parents.is_empty()
            && parents
                .iter()
                .zip(&children)
                .all(|(p, c)| (p.0 - c.0).abs() <= tol && (p.1 - c.1).abs() <= tol);
        if !same {
            return Err(refuse(
                Some(i),
                "its bend is sharp, and the pieces a hole across it leaves of its two tangent lines do not pair up along the bend",
            ));
        }
        let n = parents.len();
        for (j, (p, c)) in parents.iter().zip(&children).enumerate() {
            // The sector at each end of the stretch: the bend's own end,
            // or the side of a hole across it (named after the hole's
            // piece there).
            let side = |u: f64| -> Result<SharpFace, KernelError> {
                sides
                    .iter()
                    .find(|s| s.bend == i && (s.u - u).abs() <= tol)
                    .map(|s| SharpFace::Side(j, s.key))
                    .ok_or_else(|| {
                        refuse(
                            Some(i),
                            "its bend is sharp, and a stretch of it ends where no hole across it does",
                        )
                    })
            };
            let ends = [
                if j == 0 {
                    SharpFace::End(0)
                } else {
                    side(p.0)?
                },
                if j + 1 == n {
                    SharpFace::End(1)
                } else {
                    side(p.1)?
                },
            ];
            let skin = if j == 0 {
                SharpFace::Skin
            } else {
                SharpFace::SkinPiece(j)
            };
            let span = if n == 1 { None } else { Some((p.0, p.1)) };
            self.stretch(
                i,
                bend,
                sheet,
                (p.2, c.2),
                span,
                skin,
                ends,
                flats,
                plane,
                t,
                tol,
            )?;
        }
        Ok(())
    }

    /// Joins the stretch `span` along the axis (the whole bend: `None`)
    /// of sharp bend `i` between the rims along the parent's and the
    /// child's tangent edges `edges`: its outer skin `skin` and its end
    /// sectors `ends`.
    #[allow(clippy::too_many_arguments)]
    fn stretch(
        &mut self,
        i: usize,
        bend: &Bend,
        sheet: &Body,
        edges: (usize, usize),
        span: Option<(f64, f64)>,
        skin: SharpFace,
        ends: [SharpFace; 2],
        flats: &[Flat],
        plane: &Frame,
        t: f64,
        tol: f64,
    ) -> Result<(), KernelError> {
        let up = bend.angle > 0.0;
        let face_of = |e: usize| -> Result<FaceId, KernelError> {
            let id = EdgeId(u32::try_from(e).map_err(|_| refuse(Some(i), "too many edges"))?);
            sheet
                .faces
                .iter()
                .position(|f| {
                    f.loops
                        .iter()
                        .filter_map(|l| sheet.loop_(*l))
                        .any(|l| l.coedges.iter().any(|c| c.edge == id))
                })
                .and_then(|f| u32::try_from(f).ok())
                .map(FaceId)
                .ok_or_else(|| refuse(Some(i), "a sharp bend's flat has no face"))
        };
        let (rp, qp) = self.quad(i, edges.0, face_of(edges.0)?, up)?;
        let (rc, qc) = self.quad(i, edges.1, face_of(edges.1)?, up)?;

        // The bend's axis and its outer skin, as the folded sheet has a
        // bend's (the axis `T` above the tangent line for a bend up, on it
        // for a bend down).
        let parent = flats
            .get(bend.parent)
            .ok_or_else(|| refuse(Some(i), "a bend's parent is missing"))?;
        let wp = world(plane, parent);
        let d = unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?;
        let len = distance(bend.start, bend.end);
        let axis = wp
            .vector(lift(d))
            .normalized()
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let (zc, zo) = if up { (t, 0.0) } else { (0.0, t) };
        let c0 = wp.point(Vec3::new(bend.start[0], bend.start[1], zc));
        let p0 = wp.point(Vec3::new(bend.start[0], bend.start[1], zo));
        let frame = Frame::new(c0, axis * (-bend.angle.signum()), p0 - c0)
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let theta = bend.theta();
        let (sn, cs) = theta.psin_cos();
        let (ua, ub) = span.unwrap_or((0.0, len));
        let at = |j: usize| -> Result<Frame, KernelError> {
            frame
                .at(c0 + axis * (if j == 0 { ua } else { ub }))
                .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))
        };
        let far = |f: &Frame| f.origin() + (f.x() * cs + f.y() * sn) * t;
        let near = |f: &Frame| f.origin() + f.x() * t;
        let lost = || {
            refuse(
                Some(i),
                "the plates' corners at a sharp bend do not lie where its skins end (within the tolerance)",
            )
        };
        let frames = [at(0)?, at(1)?];
        let mut vp = [VertexId(0); 2];
        let mut vc = [VertexId(0); 2];
        let mut apex = [VertexId(0); 2];
        let mut apex_c = [VertexId(0); 2];
        for (j, f) in frames.iter().enumerate() {
            let slot = |a: &mut [VertexId; 2], v: VertexId| {
                if let Some(s) = a.get_mut(j) {
                    *s = v;
                }
            };
            slot(
                &mut vp,
                self.nearest(qp.outer, near(f), tol).ok_or_else(lost)?,
            );
            slot(
                &mut vc,
                self.nearest(qc.outer, far(f), tol).ok_or_else(lost)?,
            );
            slot(
                &mut apex,
                self.nearest(qp.inner, f.origin(), tol).ok_or_else(lost)?,
            );
            slot(
                &mut apex_c,
                self.nearest(qc.inner, f.origin(), tol).ok_or_else(lost)?,
            );
        }
        if vp[0] == vp[1] || vc[0] == vc[1] || apex[0] == apex[1] || apex_c[0] == apex_c[1] {
            return Err(lost());
        }
        // The child's inner edge goes into the parent's: both are the
        // axis between the bend's ends.
        let start_of = |e: EdgeId| self.body.edge(e).map(|e| self.vertex(e.start));
        let jp = apex
            .iter()
            .position(|v| Some(*v) == start_of(qp.inner.edge));
        let jc = apex_c
            .iter()
            .position(|v| Some(*v) == start_of(qc.inner.edge));
        let (Some(jp), Some(jc)) = (jp, jc) else {
            return Err(lost());
        };
        let flip = jp != jc;
        self.edge_alias
            .insert(qc.inner.edge.0, (qp.inner.edge.0, flip));
        for j in 0..2 {
            let (Some(from), Some(to)) = (apex_c.get(j), apex.get(j)) else {
                return Err(lost());
            };
            if from != to {
                self.vertex_alias.insert(from.0, to.0);
            }
        }

        // The arcs of the outer skin at both ends, from the parent's
        // vertex to the child's.
        let mut arcs = [EdgeId(0); 2];
        for (j, f) in frames.iter().enumerate() {
            let (Some(&a), Some(&b)) = (vp.get(j), vc.get(j)) else {
                return Err(lost());
            };
            let circle = Circle::new(*f, t)?;
            let e = self
                .body
                .add_edge(Curve3::Circle(circle), (0.0, theta), a, b);
            if let Some(s) = arcs.get_mut(j) {
                *s = e;
            }
        }
        // The outer skin: the parent's outer edge as its rim ran it, the
        // arc on to the child's, the child's outer edge as its rim ran it,
        // the other arc back.
        let (_, pa) = self.ends(qp.outer).ok_or_else(lost)?;
        let a = vp.iter().position(|v| *v == pa).ok_or_else(lost)?;
        let b = 1 - a;
        let arc = |j: usize, reversed: bool| -> Result<Coedge, KernelError> {
            Ok(Coedge {
                edge: *arcs.get(j).ok_or_else(lost)?,
                reversed,
            })
        };
        let outer = [qp.outer, arc(a, false)?, qc.outer, arc(b, true)?];
        self.closes(&outer).ok_or_else(|| {
            refuse(
                Some(i),
                "the outer skin of a sharp bend does not close round its plates' edges",
            )
        })?;
        let skin_frame = frame;
        let loop_ = self.body.add_loop(outer.to_vec());
        self.body.add_face(
            Surface::Cylinder(Cylinder::new(skin_frame, t)?),
            false,
            vec![loop_],
        );
        self.joined_face(i, skin)?;

        // The two end sectors: the arc the other way, the parent's side and
        // the child's side as their rims ran them.
        for (j, f) in frames.iter().enumerate() {
            let side_at = |q: &Quad, v: VertexId| -> Result<Coedge, KernelError> {
                q.sides
                    .iter()
                    .copied()
                    .find(|c| self.ends(*c).is_some_and(|(x, y)| x == v || y == v))
                    .ok_or_else(lost)
            };
            let (Some(&pv), Some(&cv)) = (vp.get(j), vc.get(j)) else {
                return Err(lost());
            };
            let sp = side_at(&qp, pv)?;
            let sc = side_at(&qc, cv)?;
            let a_use = arc(j, j == a)?;
            let order = [[a_use, sp, sc], [a_use, sc, sp]]
                .into_iter()
                .find(|o| self.closes(o).is_some())
                .ok_or_else(|| {
                    refuse(
                        Some(i),
                        "an end of a sharp bend does not close round its plates' edges",
                    )
                })?;
            let out = if j == 0 { -axis } else { axis };
            let end_frame = Frame::new(f.origin(), out, f.x())
                .ok_or_else(|| refuse(Some(i), "a bend's end is not finite"))?;
            let loop_ = self.body.add_loop(order.to_vec());
            self.body
                .add_face(Surface::Plane(Plane::new(end_frame)), false, vec![loop_]);
            self.joined_face(i, *ends.get(j).ok_or_else(lost)?)?;
        }
        self.dropped.insert(rp);
        self.dropped.insert(rc);
        Ok(())
    }

    /// Whether the coedges run end to end round a loop.
    fn closes(&self, coedges: &[Coedge]) -> Option<()> {
        let n = coedges.len();
        for (k, c) in coedges.iter().enumerate() {
            let next = coedges.get((k + 1) % n)?;
            let (_, end) = self.ends(*c)?;
            let (start, _) = self.ends(*next)?;
            if end != start {
                return None;
            }
        }
        Some(())
    }

    /// The body without the rims taken out, every alias resolved, the
    /// entities left numbered in their order.
    fn finish(self) -> Result<Plate, KernelError> {
        let mut out = Body::new(BodyKind::Solid);
        let mut faces = Vec::new();
        let mut made = Vec::new();
        let mut loops: Vec<Vec<Vec<Coedge>>> = Vec::new();
        let mut used_edges: BTreeSet<u32> = BTreeSet::new();
        for (f, face) in self.body.faces.iter().enumerate() {
            if self.dropped.contains(&f) {
                continue;
            }
            let mut ls = Vec::new();
            for l in &face.loops {
                let cs: Vec<Coedge> = self
                    .body
                    .loop_(*l)
                    .map(|l| l.coedges.iter().map(|c| self.coedge(*c)).collect())
                    .unwrap_or_default();
                used_edges.extend(cs.iter().map(|c| c.edge.0));
                ls.push(cs);
            }
            loops.push(ls);
            faces.push(f);
            made.push(
                *self
                    .made
                    .get(f)
                    .ok_or_else(|| refuse(None, "a face's origin is missing"))?,
            );
        }
        let mut used_vertices: BTreeSet<u32> = BTreeSet::new();
        for e in &used_edges {
            let edge = self
                .body
                .edges
                .get(*e as usize)
                .ok_or_else(|| refuse(None, "an edge is missing"))?;
            used_vertices.insert(self.vertex(edge.start).0);
            used_vertices.insert(self.vertex(edge.end).0);
        }
        let mut vmap: BTreeMap<u32, VertexId> = BTreeMap::new();
        for v in &used_vertices {
            let p = self
                .body
                .vertices
                .get(*v as usize)
                .ok_or_else(|| refuse(None, "a vertex is missing"))?
                .point;
            vmap.insert(*v, out.add_vertex(p));
        }
        let mut emap: BTreeMap<u32, EdgeId> = BTreeMap::new();
        for e in &used_edges {
            let edge = self
                .body
                .edges
                .get(*e as usize)
                .ok_or_else(|| refuse(None, "an edge is missing"))?;
            let (Some(&s), Some(&t)) = (
                vmap.get(&self.vertex(edge.start).0),
                vmap.get(&self.vertex(edge.end).0),
            ) else {
                return Err(refuse(None, "a vertex is missing"));
            };
            emap.insert(*e, out.add_edge(edge.curve.clone(), edge.range, s, t));
        }
        let mut ids = Vec::with_capacity(faces.len());
        for (f, ls) in faces.iter().zip(loops) {
            let face = self
                .body
                .faces
                .get(*f)
                .ok_or_else(|| refuse(None, "a face is missing"))?;
            let mut lids = Vec::with_capacity(ls.len());
            for cs in ls {
                let cs = cs
                    .into_iter()
                    .map(|c| {
                        emap.get(&c.edge.0)
                            .map(|&edge| Coedge {
                                edge,
                                reversed: c.reversed,
                            })
                            .ok_or_else(|| refuse(None, "an edge is missing"))
                    })
                    .collect::<Result<Vec<_>, KernelError>>()?;
                lids.push(out.add_loop(cs));
            }
            ids.push(out.add_face(face.surface.clone(), face.reversed, lids));
        }
        out.add_shell(ids);
        Ok(Plate {
            body: out,
            faces: made,
        })
    }
}
