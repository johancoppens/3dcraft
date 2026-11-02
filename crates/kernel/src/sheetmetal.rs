//! Sheet metal: a flat base with flanges bent along straight bend lines,
//! each bend with an inner radius `R` and a K-factor `K`, folded by the
//! one fold mechanism (the linkage of [`crate::origami`]) and given its
//! thickness by the one offset mechanism (the plate of [`crate::plate`]);
//! its flat pattern measured by the one unfold mechanism
//! ([`crate::unfold::develop`] on each bend's neutral cylinder).
//! docs/design/sheetmetal.md has the method, the sources and the limits.
//!
//! **The flat pattern** is built first, in the plane of the base's sketch
//! (its x, y; `z` along its normal, the side a bend `up` goes to). The
//! base is a closed profile (lines, arcs, circles, splines; holes
//! allowed). A flange sits on a straight edge: one of the base's outer
//! edges, or the far edge (the *tip*) of an earlier flange. Along the
//! edge `s → e` (the parent flat on its left, `û` the unit vector to its
//! right, away from the parent) the pattern grows by the bend's strip,
//! `BA = θ (R + K T)` wide (the bend allowance: the length of the neutral
//! fibre over the angle θ; Machinery's Handbook, *Sheet metal* › *Bend
//! allowance*), and then the flange's flat, `L` long (measured from the
//! bend's tangent line) or `L − (R + T) tan(θ/2)` (measured from the
//! virtual sharp of the outside surfaces, the outside mould line).
//!
//! **Corners and reliefs.** Where two flanged edges of the base meet at a
//! corner `p` (a left turn of the outer loop), the two strips meet at `p`
//! only. A relief of size `r` cuts the base's corner: both edges end `r`
//! from `p` (so do the flanges on them), and the corner is replaced by a
//! notch from `a = p + r eₐ` to `b = p + r e_b` (`eₐ`, `e_b` the edges'
//! directions away from `p`): `rect` the parallelogram `a, a + r e_b, b`;
//! `round` the arc about `p` through the base; `tear` the far arc of the
//! circle tangent to both edges at `a` and `b`. A corner where the loop
//! turns right or goes straight on (the strips would overlap in the
//! pattern, or two bends would continue each other) is refused.
//!
//! **Folding.** Every flat (the base, each flange) is a body of a
//! [`Linkage`]; every bend a joint with the radius `R + K T` whose point
//! is where the strip starts on the neutral fibre (`z = T − K T` for a
//! bend up, whose inner skin is the top; `z = K T` for a bend down) and
//! whose angle is given (`+θ` up, `−θ` down): its motion is the turn
//! about the axis `R + K T` from that point after the shift back across
//! the strip by `θ (R + K T)` ([`crate::origami::Joint`]). The base stays
//! in place.
//!
//! **The part.** The folded bottom skin (`z = 0` of every flat, and per
//! bend the cylinder about the bend's axis of radius `T + R` for a bend
//! up, `R` for a bend down, over θ) is one sheet whose faces are tangent
//! along the tangent lines; [`crate::plate::plate`] gives it the
//! thickness `T` on the side its normal points to (into the material),
//! with square ends everywhere. A bend's skins are the cylinders `R` and
//! `R + T` about its axis exactly.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::KernelError;
use crate::origami::{Angle, Folded, Joint, Linkage, fold_linkage};
use crate::plate::{Plate, Side, plate};
use crate::profile::{P2, Profile, Segment, Shape, cross, distance, dot, sub};
use crate::unfold::develop;
use craft3d_geom::{Circle, Curve3, Cylinder, Line, NurbsCurve, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

mod cuts;
pub mod formed;
mod jog;
mod later;
pub mod sequence;
mod trim;
pub use cuts::Cut;
pub use jog::{JogSpec, web_length};
pub use later::{LaterCuts, later_cuts};
pub use trim::Trim;

/// The most flanges one part may have.
pub const MAX_FLANGES: usize = 1024;

/// The most cut-outs (closed profiles beside the base) one part may have.
pub const MAX_CUTOUTS: usize = 4096;

/// The narrowest piece of an edge an inset may leave beside a bend, and
/// the least gap between two bends on one edge, in tolerances: a piece
/// within a few tolerances of nothing is a rim face the plate, the
/// self-intersection check and the tessellator cannot tell from a
/// degenerate one, so an inset is either 0 (within the tolerance) or at
/// least this many tolerances, never a sliver in between.
pub const SLIVER_TOLERANCES: f64 = 4.0;

/// Refuses an inset `x` that is more than the tolerance (not 0) but less
/// than [`SLIVER_TOLERANCES`] tolerances: it would leave a sliver.
fn sliver(i: usize, which: &str, x: f64, tol: f64) -> Result<(), KernelError> {
    let floor = SLIVER_TOLERANCES * tol;
    if x > tol && x < floor {
        return Err(refuse(
            Some(i),
            format!(
                "its {which} inset {x} leaves a piece of the edge narrower than {SLIVER_TOLERANCES} tolerances ({floor}) beside the bend: make it 0 or at least {floor}"
            ),
        ));
    }
    Ok(())
}

/// Which way a bend turns the flange: towards the side the base's normal
/// points to, or away from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// The straight edge a flange sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// The edge of the base made by the sketch curve with this id: on its
    /// outer loop, or on a hole's (the flange then grows into the hole).
    Base(u64),
    /// The tip (the far edge) of the flange at this index.
    Tip(usize),
    /// A side of the flange at this index: its edge at the start of its
    /// bend (0) or at the end (1).
    Side(usize, usize),
    /// The straight segment of the own outline of the flange at this index
    /// made by the outline's sketch curve with this id (not the line on
    /// its edge, which is its bend's tangent line).
    Line(usize, u64),
    /// The edge of the cut-out at this index of [`Spec::cutouts`] made by
    /// its sketch curve with this id: the cut-out is a hole of the flat it
    /// lies in (the base's or a flange's, laid out before this flange), and
    /// the flange grows into it, as on a hole of the base: a tab or a
    /// louvre bent out of a wall.
    Cutout(usize, u64),
}

/// Where a flange's length is measured from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthFrom {
    /// From the bend's tangent line: the flange's flat is this long.
    Tangent,
    /// From the virtual sharp of the outside surfaces (the outside mould
    /// line): the flat is shorter by the outside setback `(R + T) tan(θ/2)`.
    Mould,
}

/// The shape of a corner relief.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliefKind {
    Rect,
    Round,
    Tear,
}

impl ReliefKind {
    pub fn name(self) -> &'static str {
        match self {
            ReliefKind::Rect => "rect",
            ReliefKind::Round => "round",
            ReliefKind::Tear => "tear",
        }
    }
}

/// A corner relief: its shape and size (how far along each edge from the
/// corner it reaches).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relief {
    pub kind: ReliefKind,
    pub size: f64,
}

/// How a corner where two flanged edges of the base meet is closed: the
/// flanges' flats run on sideways past where their bends end, to planes
/// the folded walls give (docs/design/sheetmetal.md › *Closed corners*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerKind {
    /// Not closed: each flange ends where its bend does.
    Open,
    /// The covering flange runs to the gap short of the other's outside
    /// face; the other runs to the gap short of the covering one's inside
    /// face.
    Butt,
    /// The covering flange runs to the other's outside face (flush,
    /// covering its end); the other runs to the gap short of the covering
    /// one's inside face.
    Overlap,
    /// Both run to the plane that halves the angle between the walls, each
    /// half the gap short of it.
    Miter,
}

impl CornerKind {
    pub fn name(self) -> &'static str {
        match self {
            CornerKind::Open => "open",
            CornerKind::Butt => "butt",
            CornerKind::Overlap => "overlap",
            CornerKind::Miter => "miter",
        }
    }
}

/// A corner's treatment: its kind, the gap between the two flats, and
/// (for butt and overlap) the flange that covers the corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Closing {
    pub kind: CornerKind,
    pub gap: f64,
    /// Butt and overlap: the flange that runs over the corner; `None` the
    /// one on the edge that arrives at the corner (the base's outer loop
    /// runs counter-clockwise). Resolved in [`Corner::closing`].
    pub over: Option<usize>,
}

impl Closing {
    /// The corner left open.
    pub const OPEN: Closing = Closing {
        kind: CornerKind::Open,
        gap: 0.0,
        over: None,
    };
}

/// The treatment of the one corner where the flanges `flanges` meet (in
/// either order), in place of the part's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerSpec {
    pub flanges: [usize; 2],
    pub closing: Closing,
}

/// One flange as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlangeSpec {
    pub on: On,
    pub length: f64,
    /// The bend angle θ in radians, in (0, π]: 0 is flat, π folded back.
    pub angle: f64,
    pub direction: Direction,
    /// The inner radius `R` of the bend.
    pub radius: f64,
    /// The K-factor: the neutral fibre's distance from the inner surface
    /// over the thickness, in [0, 1].
    pub k: f64,
    pub from: LengthFrom,
    /// How far the bend's ends stand back from the start and the end of
    /// its edge (0: the bend spans the whole edge).
    pub inset: [f64; 2],
    /// The flange's flat as a closed profile of its own (holes allowed),
    /// drawn in the pattern's axes as if the bend had no width: one line of
    /// its outer loop lies on the edge's line, inside the edge, and the
    /// rest beyond it. The bend spans that line (so the insets are its);
    /// the flat is laid out the bend allowance further on. `None`: the
    /// rectangle `length` long.
    pub outline: Option<Profile>,
}

/// A sheet-metal part as the caller gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// The base flat, in the axes of `plane`.
    pub base: Profile,
    /// The plane of the base (its normal: where `up` goes).
    pub plane: Frame,
    pub thickness: f64,
    pub flanges: Vec<FlangeSpec>,
    /// The relief at every corner where two flanged edges of the base meet.
    pub relief: Option<Relief>,
    /// The relief at every end of a bend that lies inside its edge (an
    /// inset flange's, a tab's): a notch of its shape `size` deep into the
    /// parent flat beside the bend's end ([`BendEnd`]).
    pub bend_relief: Option<Relief>,
    /// How every corner where two flanged edges of the base meet is closed
    /// ([`Closing::OPEN`]: not) ...
    pub corner: Closing,
    /// ... but the corners listed here, each named by its two flanges.
    pub corners: Vec<CornerSpec>,
    /// Per flange, its id: what the pieces of an edge split at its bend
    /// are named by, so the name stays with the flange when others are
    /// added, deleted or moved in the list (each above 0, below 2³², none
    /// twice). Empty: each flange's place in the list plus 1.
    pub ids: Vec<u64>,
    /// Holes and cut-outs drawn in the pattern's axes outside the base:
    /// each closed profile becomes a hole of the one flat (a flange's; the
    /// base's holes are its own) it lies wholly inside in the flat pattern.
    pub cutouts: Vec<Cutout>,
    /// Jogs across flats (docs/design/sheetmetal.md › *Jogs across a
    /// flat*): jog `g`'s bends are the part's bends `F + 2g` and
    /// `F + 2g + 1` (`F` the flanges' count), after every flange's, and its
    /// web and far side the flats after them.
    pub jogs: Vec<JogSpec>,
}

/// A hole or cut-out drawn beside the base: a closed profile in the
/// pattern's axes, and where the caller has it from, for messages
/// ("sketch `Vents`").
#[derive(Clone, Debug, PartialEq)]
pub struct Cutout {
    pub profile: Profile,
    pub from: String,
}

/// A bend of the part, in the pattern's axes (the plane's x, y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bend {
    /// The flat it starts from (0 is the base, `i + 1` flange `i`'s) and
    /// the flat it turns (`flange + 1`).
    pub parent: usize,
    pub child: usize,
    /// The bend angle, signed: positive up.
    pub angle: f64,
    pub radius: f64,
    pub k: f64,
    pub thickness: f64,
    /// The bend allowance `|θ| (R + K T)`: the strip's width.
    pub allowance: f64,
    /// The tangent line on the parent's side, from `start` to `end` with
    /// the parent on its left.
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// Unit, square to the tangent line, towards the flange.
    pub across: [f64; 2],
    /// The length of the flange's flat beyond the strip (with an outline
    /// of its own, the farthest its outline reaches from the strip).
    pub flat_length: f64,
    /// Its start (0) and end (1) cut by a closed corner's end plane
    /// (docs/design/sheetmetal.md › *Trimming a bend*), or square.
    pub trim: [Option<Trim>; 2],
}

/// What a segment of a flat's outline is, as four numbers that stay the
/// same when flanges are added elsewhere: `[1, curve, piece, part]` a
/// piece of a sketch curve of the base (`part` 0 for the whole edge;
/// when the edge is split at the bends on it, `from · 2³² + to` with
/// `from` 0 at the edge's start or the id of the flange whose bend it
/// follows and `to` 0 at its end or the id of the one it runs to, so a
/// flange added or deleted on the
/// edge renames only the piece it splits or joins),
/// `[2, a, b, j]` piece `j` of the relief at
/// the corner of flanges `a` and `b`, `[3, i, side, part]` a side of
/// flange `i`'s rectangle (0 its tangent line, 1 the side at its bend's
/// start, 2 its tip, 3 the side at its end), `[4, i, curve, piece]` a
/// curve of flange `i`'s own outline, `[5, flat, hole, segment]` a hole
/// placed in a flat. A flange (`a`, `b`, `i`) is numbered by its id less
/// 1 and a flat by 0 or its flange's id ([`Part::flange_number`]), so the
/// numbers follow the flanges, not their places in the list.
pub type SegId = [u64; 4];

/// Where a bend starts: the segment of its parent flat's outer loop (or
/// of a hole of it) that is its tangent line on the parent's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub bend: usize,
    pub hole: Option<usize>,
    pub seg: usize,
}

impl Bend {
    /// The angle's size θ.
    pub fn theta(&self) -> f64 {
        self.angle.abs()
    }

    /// The radius of the neutral fibre, `R + K T`.
    pub fn neutral(&self) -> f64 {
        self.radius + self.k * self.thickness
    }

    /// The outside setback `(R + T) tan(θ/2)`: from a tangent line to the
    /// virtual sharp of the outside surfaces. `None` within the angular
    /// tolerance `angular` of θ = π, where the outside surfaces never meet
    /// (the rule a length to the mould line is refused by).
    pub fn setback(&self, angular: f64) -> Option<f64> {
        let half = 0.5 * self.theta();
        (half < 0.5 * PI - angular).then(|| (self.radius + self.thickness) * half.ptan())
    }

    /// The bend deduction `2 (R + T) tan(θ/2) − BA` (DIN 6935's
    /// Ausgleichswert with the sign turned): how much shorter the flat is
    /// than the two flanges measured to the virtual sharp.
    pub fn deduction(&self, angular: f64) -> Option<f64> {
        self.setback(angular).map(|s| 2.0 * s - self.allowance)
    }

    /// The bend's centre line in the pattern: across the middle of its
    /// strip.
    pub fn centre_line(&self) -> ([f64; 2], [f64; 2]) {
        let h = 0.5 * self.allowance;
        let shift = |p: P2| [p[0] + h * self.across[0], p[1] + h * self.across[1]];
        (shift(self.start), shift(self.end))
    }

    /// The tangent line on the flange's side.
    pub fn far_line(&self) -> ([f64; 2], [f64; 2]) {
        let h = self.allowance;
        let shift = |p: P2| [p[0] + h * self.across[0], p[1] + h * self.across[1]];
        (shift(self.start), shift(self.end))
    }
}

/// A flat of the pattern: its outline in the pattern's axes, and the
/// motion that folds it (in the pattern's axes, `z` along the normal).
#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    pub outer: Vec<Segment>,
    pub holes: Vec<Vec<Segment>>,
    pub transform: Transform,
    /// A flange's flat: the place of its tip (the far edge, `b0 → b1`) in
    /// `outer`; `None` for the base.
    pub tip: Option<usize>,
    /// The ends of the flat's extensions at closed corners: the place of
    /// each end's segment in `outer`, and the plane (in the pattern's axes
    /// as folded) its edge of the part lies on.
    pub ends: Vec<End>,
    /// Per segment of `outer`, its place in the flat's outline with every
    /// corner open (an extension's end stands for the side it replaces),
    /// or `None` for a piece an extension adds (along its tangent line or
    /// its tip): what keeps the part's names when a corner is closed.
    pub was: Vec<Option<usize>>,
    /// The bends that start from this flat's edges.
    pub seats: Vec<Seat>,
    /// Per segment of the outline with every corner open (as `was`
    /// counts them), and per segment of each hole, what it is, the same
    /// whatever else the part has ([`edge_numbers`]).
    pub ids: Vec<SegId>,
    pub hole_ids: Vec<Vec<SegId>>,
    /// Whether the flat is a flange's outline of its own (not the
    /// rectangle of its length).
    pub outlined: bool,
    /// Per segment of the outer loop as laid out, before it was split at
    /// the bends on it, the place of its first piece in `outer` with every
    /// corner open, and that outline's length last: a rectangle's sides
    /// are `pieces[k]..pieces[k + 1]`.
    pub pieces: Vec<usize>,
    /// Per segment of `outer`, for a piece a closed corner's extension
    /// adds (`was` is `None`), the number it is named by
    /// ([`EdgeKey::Piece`]): its place as if each side of the rectangle
    /// were one segment, so a flange on the tip renumbers none (empty:
    /// none added).
    pub added: Vec<Option<usize>>,
    /// Per segment of `outer`, for a piece of a bend relief's notch that
    /// a closed corner's extension carries (a tip flange whose bend
    /// reaches the tip's end the extension runs on past), the bend, its
    /// end (0 at its start, 1 at its end) and the piece's place in the
    /// notch: named `[11, b, end, j]` as a notch cut by
    /// [`split_at_seats`] is (empty: none).
    pub notches: Vec<Option<[usize; 3]>>,
}

/// What an edge of the folded sheet is, the same whether the part's
/// corners are open or closed ([`Part::edge_keys`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKey {
    /// Segment `seg` of flat `flat`'s outline with every corner open.
    Outline { flat: usize, seg: usize },
    /// A piece an extension adds: segment `seg` of the closed outline.
    Piece { flat: usize, seg: usize },
    /// Segment `seg` of hole `hole` of flat `flat`.
    Hole {
        flat: usize,
        hole: usize,
        seg: usize,
    },
    /// The arc at the start (0) or the end (1) of bend `bend`.
    Arc { bend: usize, end: usize },
    /// Piece `piece` of the bend relief's notch at the start (0) or the
    /// end (1) of bend `bend`, cut into a closed corner's extension.
    Notch {
        bend: usize,
        end: usize,
        piece: usize,
    },
    /// Piece `piece` of segment `seg` of flat `flat`'s outline: a tangent
    /// line a hole across its bend crosses.
    Split {
        flat: usize,
        seg: usize,
        piece: usize,
    },
    /// The end plane's cut of the bottom skin at the start (0) or the end
    /// (1) of bend `bend`, trimmed by a closed corner (the rest of that
    /// end is still its arc).
    Trim { bend: usize, end: usize },
    /// Piece `piece` of segment `seg` of hole `hole` of flat `flat`: the
    /// tangent line of a flange on the hole's edge that a hole across its
    /// bend crosses.
    SplitHole {
        flat: usize,
        hole: usize,
        seg: usize,
        piece: usize,
    },
    /// Piece `piece` of segment `seg` of cut `cut` (a hole across a bend).
    Cut {
        cut: usize,
        seg: usize,
        piece: usize,
    },
}

/// The end of a flange's extension at a closed corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct End {
    /// The place of the end's segment in the flat's outline.
    pub segment: usize,
    /// The plane the end face lies on, folded, in the pattern's axes: a
    /// point of it and its unit normal.
    pub point: Vec3,
    pub normal: Vec3,
}

/// An end of a bend that lies inside its edge (the edge runs on past it:
/// an inset flange, a tab), where the parent flat would turn into the
/// bend's end at a concave corner of 270 degrees without a bend relief.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BendEnd {
    pub bend: usize,
    /// 0 the bend's start, 1 its end.
    pub end: usize,
    /// The tangent line's end, in the pattern's axes.
    pub point: [f64; 2],
    /// The relief cut there ([`Spec::bend_relief`]), if any.
    pub relief: Option<Relief>,
}

/// A corner of the base where two flanged edges meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub point: [f64; 2],
    /// The bends on the edge that arrives at the corner and on the one
    /// that leaves it.
    pub bends: [usize; 2],
    pub relief: Option<Relief>,
    /// How it is closed; for butt and overlap `over` is the covering
    /// flange.
    pub closing: Closing,
}

/// What a face of the folded sheet is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetFace {
    Flat(usize),
    Bend(usize),
    /// The further faces (1, 2, ...) a flat or a bend is left in by a hole
    /// across a bend (a slot across a whole strip leaves the bend in two).
    FlatPiece(usize, usize),
    BendPiece(usize, usize),
}

/// A sheet-metal part: its pattern, its folded bottom skin and the solid.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub plane: Frame,
    pub thickness: f64,
    pub flats: Vec<Flat>,
    pub bends: Vec<Bend>,
    pub corners: Vec<Corner>,
    /// Every end of a bend inside its edge, with its relief if it has one.
    pub bend_ends: Vec<BendEnd>,
    /// The holes across bends: cut-outs that lie in no one flat but in the
    /// pattern, crossing tangent lines (docs/design/sheetmetal.md › *Holes
    /// across a bend*).
    pub cuts: Vec<Cut>,
    /// The folded bottom skin: one face per flat and per bend (and the
    /// further faces a hole across a bend leaves).
    pub sheet: Body,
    /// Per edge of `sheet`, what it is.
    pub edge_keys: Vec<EdgeKey>,
    pub sheet_faces: Vec<SheetFace>,
    pub solid: Plate,
    /// Per flange, its id ([`Spec::ids`], or its place plus 1 when none
    /// are given): what every name of the part that belongs to a flange is
    /// numbered by ([`Part::flange_number`]), so deleting, inserting or
    /// moving a flange in the list retargets no name.
    pub marks: Vec<u64>,
    /// How many of the bends are flanges' (the spec's flanges, in their
    /// order); the rest are jogs', two per jog ([`Spec::jogs`]).
    pub flanges: usize,
}

/// The highest flange id: a flange's faces are numbered `2 · id − 1` and
/// `2 · id` (*Names*), which must stay below 2³² (the further pieces a
/// hole across a bend leaves are numbered above it).
pub const MAX_FLANGE_ID: u64 = (1 << 31) - 1;

impl Part {
    /// The number flange `i`'s names carry: its id less 1, which is its
    /// place in the list for a part whose ids are its places plus 1 (so
    /// such a part is named as before ids were kept).
    pub fn flange_number(&self, i: usize) -> Option<u64> {
        flange_number(&self.marks, i)
    }

    /// The number flat `f`'s names carry: 0 the base, flange `f − 1`'s id
    /// for a flange's flat.
    pub fn flat_number(&self, f: usize) -> Option<u64> {
        flat_number(&self.marks, f)
    }
}

/// `None` for a flange with no id (no number is made up for it, which
/// could be another flange's).
fn flange_number(marks: &[u64], i: usize) -> Option<u64> {
    marks.get(i)?.checked_sub(1)
}

fn flat_number(marks: &[u64], f: usize) -> Option<u64> {
    match f.checked_sub(1) {
        None => Some(0),
        Some(i) => marks.get(i).copied().filter(|&m| m > 0),
    }
}

fn refuse(flange: Option<usize>, reason: impl Into<String>) -> KernelError {
    KernelError::SheetMetal {
        flange,
        reason: reason.into(),
    }
}

fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

fn unit(a: P2) -> Option<P2> {
    let l = a[0].phypot(a[1]);
    (l.is_finite() && l > 0.0).then(|| [a[0] / l, a[1] / l])
}

fn lift(p: P2) -> Vec3 {
    Vec3::new(p[0], p[1], 0.0)
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

/// Builds the part `spec` describes (see the module documentation).
///
/// **Errors.** [`KernelError::SheetMetal`] with the flange it is about and
/// why: a size that is not one (a thickness or a length not more than the
/// tolerance, a radius below zero, a K-factor outside [0, 1], an angle
/// outside (0, 180] degrees), a radius within the tolerance of zero (a
/// sharp bend, unsupported), an edge that is not straight or carries two
/// flanges, a flange on the tip of a later one, flanged edges that meet
/// at a reflex corner or in line, a relief that does not fit its edges;
/// and whatever the fold or the plate refuses.
pub fn build(spec: &Spec, tolerance: &Tolerance) -> Result<Part, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let t = spec.thickness;
    if !(t.is_finite() && t > tol) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: t,
        });
    }
    if spec.flanges.len() > MAX_FLANGES {
        return Err(refuse(
            None,
            format!("a part takes at most {MAX_FLANGES} flanges"),
        ));
    }
    for (i, f) in spec.flanges.iter().enumerate() {
        check_flange(i, f, tol)?;
    }
    let marks = flange_marks(spec)?;
    for (what, relief) in [("relief", spec.relief), ("bend relief", spec.bend_relief)] {
        if let Some(relief) = relief
            && !(relief.size.is_finite() && relief.size > tol)
        {
            return Err(refuse(
                None,
                format!(
                    "a {what}'s size has to be more than the tolerance, got {}",
                    relief.size
                ),
            ));
        }
    }

    check_closing("corner", &spec.corner, t, tol)?;
    if spec.corners.len() > MAX_FLANGES {
        return Err(refuse(
            None,
            format!("a part takes at most {MAX_FLANGES} corner treatments"),
        ));
    }
    for (i, c) in spec.corners.iter().enumerate() {
        check_closing(&format!("corners.{i}"), &c.closing, t, tol)?;
    }

    // Where each flange sits: an edge of the base (of its outer loop or of
    // a hole's) by its sketch curve, or an earlier flange's tip or side.
    let outer = &spec.base.outer.segments;
    let n = outer.len();
    // Per loop of the base (`None` the outer one, `Some(h)` hole `h`) and
    // segment of it, the flanges on it, in order.
    let mut on_base: BTreeMap<(Option<usize>, usize), Vec<usize>> = BTreeMap::new();
    for (i, f) in spec.flanges.iter().enumerate() {
        match f.on {
            On::Base(id) => {
                let found: Vec<(Option<usize>, usize)> = std::iter::once((None, outer))
                    .chain(
                        spec.base
                            .holes
                            .iter()
                            .enumerate()
                            .map(|(h, l)| (Some(h), &l.segments)),
                    )
                    .flat_map(|(h, segs)| {
                        segs.iter()
                            .enumerate()
                            .filter(|(_, s)| s.source == id)
                            .map(move |(k, _)| (h, k))
                    })
                    .collect();
                let (h, k) = match found.as_slice() {
                    [] => {
                        return Err(refuse(
                            Some(i),
                            format!(
                                "the curve {id} is no edge of the base's outline or of a hole of it"
                            ),
                        ));
                    }
                    [one] => *one,
                    _ => {
                        return Err(refuse(
                            Some(i),
                            format!(
                                "the curve {id} is cut into several edges of the base; a flange needs one straight edge"
                            ),
                        ));
                    }
                };
                let seg = match h {
                    None => outer.get(k),
                    Some(h) => spec.base.holes.get(h).and_then(|l| l.segments.get(k)),
                };
                if !matches!(seg.map(|s| &s.shape), Some(Shape::Line)) {
                    return Err(refuse(
                        Some(i),
                        if h.is_some() {
                            format!(
                                "the curve {id} bounds a hole of the base and is not straight; a flange sits on a straight edge (on a curved edge it would be a stretched or shrunk flange, not a bend: not supported)"
                            )
                        } else {
                            format!(
                                "the curve {id} is not straight; a flange sits on a straight edge (a flange on a curved edge is not a bend but a stretched or shrunk flange: its bend region is a piece of a torus, which is not developable, so it has no flat pattern that keeps lengths; not supported)"
                            )
                        },
                    ));
                }
                on_base.entry((h, k)).or_default().push(i);
            }
            On::Line(j, _) => {
                if j >= i {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "a flange sits on a line of an earlier flange's outline; flange {j} comes later"
                        ),
                    ));
                }
                if spec.flanges.get(j).is_some_and(|p| p.outline.is_none()) {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "flange {j} has no outline of its own (it is a rectangle): put the flange on its tip or a side"
                        ),
                    ));
                }
            }
            On::Tip(j) | On::Side(j, _) => {
                if j >= i {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "a flange sits on the tip or a side of an earlier flange; flange {j} comes later"
                        ),
                    ));
                }
                if let On::Side(_, end) = f.on
                    && end > 1
                {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "a flange's side is 0 (at the start of its bend) or 1 (at its end), got {end}"
                        ),
                    ));
                }
                if spec.flanges.get(j).is_some_and(|p| p.outline.is_some()) {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "flange {j} has an outline of its own, whose tip and sides are not the rectangle's: put the flange on one of its lines (`line`)"
                        ),
                    ));
                }
            }
            On::Cutout(c, curve) => {
                cutout_edge(spec, i, c, curve)?;
            }
        }
    }

    // Corners between two flanged edges, and the base's outline with its
    // reliefs. A corner is where two flanges reach the point two edges of
    // the base's outer loop share (an inset flange does not reach it; a
    // flange with an outline of its own reaches it when its outline's line
    // on the edge ends there, or a relief's size short of it with a
    // relief, where the relief shortens the edge).
    let gap = spec.relief.map_or(0.0, |relief| relief.size);
    let reaches = |i: usize, k: usize, end: usize| {
        spec.flanges.get(i).is_some_and(|f| match &f.outline {
            None => f.inset.get(end).is_some_and(|&x| x <= tol),
            Some(profile) => outer
                .get(k)
                .and_then(|seg| outline_gaps(profile, seg.start, seg.end, tol))
                .and_then(|g| g.get(end).copied())
                .is_some_and(|g| (g - gap).abs() <= tol),
        })
    };
    let reaching = |k: usize, end: usize| -> Option<usize> {
        on_base
            .get(&(None, k))?
            .iter()
            .copied()
            .find(|&i| reaches(i, k, end))
    };
    let mut corners: Vec<Corner> = Vec::new();
    let mut corner_at_end: BTreeMap<usize, usize> = BTreeMap::new();
    for k in 0..n {
        let next = (k + 1) % n;
        let (Some(fa), Some(fb)) = (reaching(k, 1), reaching(next, 0)) else {
            continue;
        };
        let (Some(a), Some(b)) = (outer.get(k), outer.get(next)) else {
            continue;
        };
        let (Some(da), Some(db)) = (unit(sub(a.end, a.start)), unit(sub(b.end, b.start))) else {
            return Err(refuse(Some(fa), "an edge with a flange has no length"));
        };
        let turn = cross(da, db);
        if !(turn > tolerance.angular) {
            return Err(refuse(
                Some(fb),
                if dot(da, db) > 0.0 && turn.abs() <= tolerance.angular {
                    format!(
                        "the flanges {fa} and {fb} sit on edges in line; make them one flange on one edge"
                    )
                } else {
                    // docs/design/sheetmetal.md › *Reflex corners*: the two
                    // strips' corners at the point overlap for any right
                    // turn, and walls that meet need each other's place in
                    // the sheet, so neither an open nor a closed corner is
                    // cut from one sheet.
                    let closed = closing_of(spec, [fa, fb])
                        .map(|c| c.kind != CornerKind::Open)
                        .unwrap_or(false);
                    format!(
                        "the flanges {fa} and {fb} meet at a reflex corner of the base: there the two flanges need the same part of the sheet (their bend strips overlap in the flat pattern at the corner{}), so the corner cannot be made from one sheet, open or closed (closed, only where one edge is shorter than the other flange's bend allowance, a tab about 2 thicknesses wide, and then only with the bends trimmed, which is not built); inset them at this corner far enough that their pieces of the sheet keep apart (at a square corner both: the strip of one that reaches the corner lies along the other's edge), or make one of them a part of its own",
                        if closed {
                            ", and a flange's flat run on to meet the other's wall lies across the other flange's piece of the sheet"
                        } else {
                            ""
                        }
                    )
                },
            ));
        }
        corner_at_end.insert(k, corners.len());
        corners.push(Corner {
            point: a.end,
            bends: [fa, fb],
            relief: spec.relief,
            closing: closing_of(spec, [fa, fb])?,
        });
    }
    for (i, c) in spec.corners.iter().enumerate() {
        let [x, y] = c.flanges;
        let meet = corners
            .iter()
            .any(|k| k.bends == [x, y] || k.bends == [y, x]);
        if !meet {
            return Err(refuse(
                None,
                format!(
                    "corners.{i}: the flanges {x} and {y} do not meet at a corner of the base (two flanges on edges of the base that follow each other at a corner)"
                ),
            ));
        }
    }
    let r = spec.relief.map_or(0.0, |relief| relief.size);
    let mut base_outer: Vec<Segment> = Vec::with_capacity(n + 2 * corners.len());
    let mut base_ids: Vec<SegId> = Vec::with_capacity(n + 2 * corners.len());
    let mut base_edge: BTreeMap<usize, usize> = BTreeMap::new();
    for (k, seg) in outer.iter().enumerate() {
        let before = (k + n - 1) % n;
        let mut s = seg.clone();
        let d = unit(sub(seg.end, seg.start));
        if spec.relief.is_some() {
            let len = distance(seg.start, seg.end);
            let cut_start = corner_at_end.contains_key(&before);
            let cut_end = corner_at_end.contains_key(&k);
            let cuts = f64::from(u8::from(cut_start)) + f64::from(u8::from(cut_end));
            if cuts > 0.0 && !(len - cuts * r > tol) {
                return Err(refuse(
                    on_base
                        .get(&(None, k))
                        .and_then(|list| list.first().copied()),
                    format!(
                        "a relief of size {r} does not fit the edge of curve {} ({len} long)",
                        seg.source
                    ),
                ));
            }
            if let Some(d) = d {
                if cut_start {
                    s.start = add(seg.start, scale(d, r));
                }
                if cut_end {
                    s.end = sub(seg.end, scale(d, r));
                }
            }
        }
        if on_base.contains_key(&(None, k)) {
            base_edge.insert(k, base_outer.len());
        }
        base_ids.push([1, seg.source, u64::from(seg.piece), 0]);
        base_outer.push(s);
        if let (Some(&c), Some(relief)) = (corner_at_end.get(&k), spec.relief) {
            let next = outer.get((k + 1) % n);
            let (Some(next), Some(d)) = (next, d) else {
                continue;
            };
            let Some(dn) = unit(sub(next.end, next.start)) else {
                continue;
            };
            let p = corners.get(c).map_or(seg.end, |c| c.point);
            let [fa, fb] = corners.get(c).map_or([0, 0], |c| c.bends);
            let pieces = notch(p, scale(d, -1.0), dn, relief)?;
            let unmarked = || refuse(None, "a flange has no id");
            let (na, nb) = (
                flange_number(&marks, fa).ok_or_else(unmarked)?,
                flange_number(&marks, fb).ok_or_else(unmarked)?,
            );
            base_ids.extend((0..pieces.len()).map(|j| [2, na, nb, j as u64]));
            base_outer.extend(pieces);
        }
    }
    let base_holes: Vec<Vec<Segment>> =
        spec.base.holes.iter().map(|h| h.segments.clone()).collect();
    if let Some(relief) = spec.relief {
        reliefs_fit(&base_outer, &base_holes, relief, tolerance)?;
    }

    // The bends and the flanges' flats, in order: a parent comes first.
    // Then the jogs' (two bends, a web and a far side each), laid out as
    // soon as the flat each crosses is: a flange may sit on a far side's
    // edge, so its parent may come after it in the list.
    let nf = spec.flanges.len();
    let total = nf + 2 * spec.jogs.len();
    let mut bends: Vec<Bend> = vec![PLACEHOLDER_BEND; total];
    let mut flats: Vec<Flat> = vec![empty_flat(); 1 + total];
    let mut laid: Vec<bool> = vec![false; 1 + total];
    // Per flat, the flat it was cut from by jogs (itself if none): where
    // the edges a flange names are looked for.
    let mut root: Vec<usize> = (0..=total).collect();
    let n_base = base_outer.len();
    if let Some(slot) = laid.first_mut() {
        *slot = true;
    }
    let base_flat = Flat {
        outer: base_outer,
        holes: base_holes,
        transform: Transform::IDENTITY,
        tip: None,
        ends: Vec::new(),
        was: (0..n_base).map(Some).collect(),
        seats: Vec::new(),
        ids: base_ids,
        hole_ids: spec
            .base
            .holes
            .iter()
            .map(|h| {
                h.segments
                    .iter()
                    .map(|g| [1, g.source, u64::from(g.piece), 0])
                    .collect()
            })
            .collect(),
        outlined: false,
        pieces: (0..=n_base).collect(),
        added: Vec::new(),
        notches: Vec::new(),
    };
    // What the flanges on the base name its edges by, before a jog cuts
    // it.
    let base_ids_drawn = base_flat.ids.clone();
    let base_hole_ids_drawn = base_flat.hole_ids.clone();
    if let Some(slot) = flats.first_mut() {
        *slot = base_flat;
    }
    // Per bend, the edge it sits on before the edges are split at the
    // insets: its flat, loop and segment.
    let mut seated: Vec<(usize, Option<usize>, usize)> = vec![(0, None, 0); total];
    // Per flat, how far it lies from where it would with every bend
    // between the base and it of no width (where an outline of a flange
    // on its edge is drawn, `own_outline`, and a jog's line).
    let mut lifts: Vec<P2> = vec![[0.0, 0.0]; 1 + total];
    for (g, jog) in spec.jogs.iter().enumerate() {
        jog::check(g, jog, tol, tolerance.angular).map_err(|e| refuse(Some(nf + 2 * g), e))?;
    }
    let mut pending: Vec<bool> = vec![true; spec.jogs.len()];
    let mut jogged: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut jogs = Jogs {
        spec,
        marks: &marks,
        nf,
        thickness: t,
        tolerance,
    };
    jogs.apply(
        &mut flats,
        &mut laid,
        &mut root,
        &mut bends,
        &mut seated,
        &mut lifts,
        &mut pending,
        &mut jogged,
    )?;
    // With a flange on a cut-out's edge, a flat that holds such a cut-out
    // takes its cut-outs as soon as it is laid out (in the order
    // `place_cutouts` takes them), so the flange finds its edge; every
    // other cut-out is placed after the corners are closed, as before.
    let early = spec.flanges.iter().any(|f| matches!(f.on, On::Cutout(..)));
    let carrying: std::collections::BTreeSet<usize> = spec
        .flanges
        .iter()
        .filter_map(|f| match f.on {
            On::Cutout(c, _) => Some(c),
            _ => None,
        })
        .collect();
    let mut placed: Vec<Option<(usize, usize)>> = vec![None; spec.cutouts.len()];
    if early {
        place_early(
            &mut flats,
            0,
            spec,
            &carrying,
            &marks,
            &mut placed,
            tolerance,
        )?;
    }
    for (i, f) in spec.flanges.iter().enumerate() {
        let lost = || refuse(Some(i), "a flange lost its edge");
        // The edge with id `id` (of an outer loop, or of a hole) among the
        // flats cut from flat `from` by jogs.
        let seek = |from: usize,
                    id: SegId,
                    in_hole: bool|
         -> Result<(usize, Option<usize>, usize), KernelError> {
            let mut found = Vec::new();
            for (q, flat) in flats.iter().enumerate() {
                if !laid.get(q).copied().unwrap_or(false) || root.get(q) != Some(&from) {
                    continue;
                }
                if in_hole {
                    for (h, ids) in flat.hole_ids.iter().enumerate() {
                        for (k, x) in ids.iter().enumerate() {
                            if *x == id {
                                found.push((q, Some(h), k));
                            }
                        }
                    }
                } else {
                    for (k, x) in flat.ids.iter().enumerate() {
                        if *x == id {
                            found.push((q, None, k));
                        }
                    }
                }
            }
            match found.as_slice() {
                [one] => Ok(*one),
                [] if jogged.iter().any(|&q| root.get(q) == Some(&from)) => Err(refuse(
                    Some(i),
                    "the edge it sits on is cut by a jog's line (a flange sits on an edge wholly on one side of a jog): move the jog's line or the flange",
                )),
                _ => Err(lost()),
            }
        };
        let rect_id = |j: usize, side: u64| -> Result<SegId, KernelError> {
            let number = flange_number(&marks, j).ok_or_else(|| refuse(Some(i), "it has no id"))?;
            Ok([3, number, side, 0])
        };
        let (parent, hole, k) = match f.on {
            On::Base(_) => {
                let &(h, k) = on_base
                    .iter()
                    .find(|(_, list)| list.contains(&i))
                    .map(|(key, _)| key)
                    .ok_or_else(lost)?;
                match h {
                    None => {
                        let at = *base_edge.get(&k).ok_or_else(lost)?;
                        let id = *base_ids_drawn.get(at).ok_or_else(lost)?;
                        seek(0, id, false)?
                    }
                    Some(h) => {
                        let id = *base_hole_ids_drawn
                            .get(h)
                            .and_then(|ids| ids.get(k))
                            .ok_or_else(lost)?;
                        seek(0, id, true)?
                    }
                }
            }
            On::Tip(j) => seek(j + 1, rect_id(j, 2)?, false)?,
            On::Side(j, end) => seek(j + 1, rect_id(j, if end == 0 { 1 } else { 3 })?, false)?,
            On::Cutout(c, curve) => {
                let Some((f, h)) = placed.get(c).copied().flatten() else {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "it sits on an edge of the cut-out {} (curve {curve}), which lies in no flat laid out before it (the base or an earlier flange's flat, wholly inside it): a flange on a cut-out's edge comes after the flange whose flat holds the cut-out, and a cut-out across a bend or past the pattern's edge carries none",
                            spec.cutouts
                                .get(c)
                                .map_or_else(|| c.to_string(), |x| x.from.clone())
                        ),
                    ));
                };
                let k = flats
                    .get(f)
                    .and_then(|fl| fl.holes.get(h))
                    .and_then(|l| l.iter().position(|s| s.source == curve))
                    .ok_or_else(lost)?;
                (f, Some(h), k)
            }
            On::Line(j, curve) => {
                // Among the flats cut from the flange's by jogs: a piece of
                // a curve a jog cuts has the curve's source too, so such a
                // curve is several edges.
                let found: Vec<(usize, usize)> = flats
                    .iter()
                    .enumerate()
                    .filter(|(q, _)| {
                        laid.get(*q).copied().unwrap_or(false) && root.get(*q) == Some(&(j + 1))
                    })
                    .flat_map(|(q, fl)| {
                        fl.outer
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.source == curve)
                            .map(move |(k, _)| (q, k))
                    })
                    .collect();
                let (q, k) = match found.as_slice() {
                    [one] => *one,
                    [] => {
                        return Err(refuse(
                            Some(i),
                            format!(
                                "the curve {curve} is no line of flange {j}'s outline (of its outer loop)"
                            ),
                        ));
                    }
                    _ => {
                        return Err(refuse(
                            Some(i),
                            format!(
                                "the curve {curve} is cut into several edges of flange {j}'s outline; a flange needs one straight edge"
                            ),
                        ));
                    }
                };
                if q == j + 1 && k == 0 {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "the curve {curve} is the line of flange {j}'s outline on its edge, its bend's tangent line: a flange sits on another line of the outline"
                        ),
                    ));
                }
                let outer = flats.get(q).map(|f| f.outer.as_slice()).unwrap_or(&[]);
                if !matches!(outer.get(k).map(|s| &s.shape), Some(Shape::Line)) {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "the curve {curve} of flange {j}'s outline is not straight; a flange sits on a straight edge"
                        ),
                    ));
                }
                (q, None, k)
            }
        };
        let edge = flats
            .get(parent)
            .and_then(|fl| match hole {
                None => fl.outer.get(k),
                Some(h) => fl.holes.get(h).and_then(|l| l.get(k)),
            })
            .ok_or_else(lost)?;
        let (s, e) = (edge.start, edge.end);
        let len = distance(s, e);
        let d = unit(sub(e, s)).ok_or_else(|| refuse(Some(i), "the edge has no length"))?;
        let across = [d[1], -d[0]];
        let theta = f.angle;
        let neutral = f.radius + f.k * t;
        let allowance = theta * neutral;
        let shift = scale(across, allowance);
        let number = flange_number(&marks, i).ok_or_else(|| refuse(Some(i), "it has no id"))?;
        let rect: Vec<SegId> = (0..4).map(|side| [3, number, side, 0]).collect();
        let (start, end, outer, holes, flat_length, (ids, hole_ids)) = match &f.outline {
            None => {
                let [i0, i1] = f.inset;
                if !(i0 + i1 <= len - SLIVER_TOLERANCES * tol) {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "the insets {i0} and {i1} leave nothing of the edge it sits on ({len} long)"
                        ),
                    ));
                }
                sliver(i, "start", i0, tol)?;
                sliver(i, "end", i1, tol)?;
                let start = if i0 <= tol { s } else { add(s, scale(d, i0)) };
                let end = if i1 <= tol { e } else { sub(e, scale(d, i1)) };
                let flat_length = match f.from {
                    LengthFrom::Tangent => f.length,
                    LengthFrom::Mould => {
                        let half = 0.5 * theta;
                        if !(half < 0.5 * PI - tolerance.angular) {
                            return Err(refuse(
                                Some(i),
                                "a length to the outside mould line needs an angle below 180 degrees (at 180 the outside surfaces never meet); measure it from the tangent line",
                            ));
                        }
                        f.length - (f.radius + t) * half.ptan()
                    }
                };
                if !(flat_length > tol) {
                    return Err(refuse(
                        Some(i),
                        format!(
                            "the flange's flat would be {flat_length} long: its length is shorter than the outside setback (R + T) tan(θ/2)"
                        ),
                    ));
                }
                let a0 = add(start, shift);
                let a1 = add(end, shift);
                let b0 = add(a0, scale(across, flat_length));
                let b1 = add(a1, scale(across, flat_length));
                (
                    start,
                    end,
                    vec![line(a1, a0), line(a0, b0), line(b0, b1), line(b1, a1)],
                    Vec::new(),
                    flat_length,
                    (rect, Vec::new()),
                )
            }
            Some(profile) => {
                if f.from == LengthFrom::Mould {
                    return Err(refuse(
                        Some(i),
                        "a flange with an outline of its own is drawn from its bend's tangent line; `from: mould` does not apply to it",
                    ));
                }
                if f.inset.iter().any(|&x| x != 0.0) {
                    return Err(refuse(
                        Some(i),
                        "a flange with an outline of its own spans the outline's line on its edge, which gives its insets; give it no insets",
                    ));
                }
                let lift = lifts.get(parent).copied().ok_or_else(lost)?;
                let (start, end, outer, holes, flat_length) =
                    own_outline(i, profile, (s, e), shift, lift, tol)?;
                // Its curves by their sketch's ids, its tangent line as a
                // rectangle's is, its holes by their place.
                let mut ids: Vec<SegId> = outer
                    .iter()
                    .map(|g| [4, number, g.source, u64::from(g.piece)])
                    .collect();
                if let Some(first) = ids.first_mut() {
                    *first = [3, number, 0, 0];
                }
                let hole_ids = holes
                    .iter()
                    .enumerate()
                    .map(|(h, l)| {
                        (0..l.len())
                            .map(|j| [5, number + 1, h as u64, j as u64])
                            .collect()
                    })
                    .collect();
                (start, end, outer, holes, flat_length, (ids, hole_ids))
            }
        };
        let signed = match f.direction {
            Direction::Up => theta,
            Direction::Down => -theta,
        };
        let slot = bends.get_mut(i).ok_or_else(lost)?;
        *slot = Bend {
            parent,
            child: i + 1,
            angle: signed,
            radius: f.radius,
            k: f.k,
            thickness: t,
            allowance,
            start,
            end,
            across,
            flat_length,
            trim: [None; 2],
        };
        let m = outer.len();
        let slot = flats.get_mut(i + 1).ok_or_else(lost)?;
        *slot = Flat {
            outer,
            holes,
            transform: Transform::IDENTITY,
            tip: f.outline.is_none().then_some(2),
            ends: Vec::new(),
            was: (0..m).map(Some).collect(),
            seats: Vec::new(),
            ids,
            hole_ids,
            outlined: f.outline.is_some(),
            pieces: (0..=m).collect(),
            added: Vec::new(),
            notches: Vec::new(),
        };
        if let Some(slot) = laid.get_mut(i + 1) {
            *slot = true;
        }
        if let Some(slot) = seated.get_mut(i) {
            *slot = (parent, hole, k);
        }
        let below = lifts.get(parent).copied().ok_or_else(lost)?;
        if let Some(slot) = lifts.get_mut(i + 1) {
            *slot = add(below, shift);
        }
        jogs.apply(
            &mut flats,
            &mut laid,
            &mut root,
            &mut bends,
            &mut seated,
            &mut lifts,
            &mut pending,
            &mut jogged,
        )?;
        if early {
            place_early(
                &mut flats,
                i + 1,
                spec,
                &carrying,
                &marks,
                &mut placed,
                tolerance,
            )?;
        }
    }
    if let Some(g) = pending.iter().position(|&p| p) {
        return Err(refuse(
            Some(nf + 2 * g),
            format!(
                "jog {g}: the middle of its line lies in no flat (drawn as if every bend had no width: on the base, or where a flange's flat lies beside its edge), or on an outline or in a hole; draw the line across one flat"
            ),
        ));
    }
    // A flat a jog cuts is no longer its rectangle: a closed corner of its
    // flange (which runs the rectangle on) is not built.
    for c in &corners {
        if c.closing.kind == CornerKind::Open {
            continue;
        }
        if let Some(&x) = c.bends.iter().find(|&&x| jogged.contains(&(x + 1))) {
            return Err(refuse(
                Some(x),
                "its corner is closed, but a jog crosses its flat; closing the corner of a jogged flange is not built (owed): open the corner, or jog the flange's tip instead (two flanges on it)",
            ));
        }
    }
    let mut bend_ends = split_at_seats(&mut flats, &bends, &seated, &marks, spec.bend_relief, tol)?;

    // The fold places every flat by its bends alone, so the closed
    // corners' extensions are laid out against the walls as folded.
    let folded = fold(&flats, &bends, tolerance)?;
    for (flat, transform) in flats.iter_mut().zip(&folded.transforms) {
        flat.transform = *transform;
    }
    let early: Vec<usize> = placed.iter().flatten().map(|&(f, _)| f).collect();
    let moved = close_corners(
        &mut flats,
        &mut bends,
        &corners,
        t,
        spec.bend_relief,
        &mut bend_ends,
        &early,
        tolerance,
    )?;
    if moved {
        // Flanges moved with a side to a closed corner's end: placed
        // again by their bends (the rest place as before, bit for bit).
        let folded = fold(&flats, &bends, tolerance)?;
        for (flat, transform) in flats.iter_mut().zip(&folded.transforms) {
            flat.transform = *transform;
        }
    }
    bend_reliefs_fit(&flats, &bends, &bend_ends, tolerance)?;
    let done: Vec<bool> = placed.iter().map(Option::is_some).collect();
    let across = place_cutouts(&mut flats, &spec.cutouts, &done, tolerance)?;
    pattern_clear(&flats, &bends, &[], &corners, tolerance)?;
    let cuts = place_cuts(&flats, &bends, &across, tolerance)?;
    let (sheet, sheet_faces, ends, edge_keys) =
        folded_sheet(spec, &flats, &bends, &cuts, tolerance, true)?;
    let faces: Vec<FaceId> = (0..sheet.faces.len())
        .filter_map(|f| u32::try_from(f).ok().map(FaceId))
        .collect();
    // An extension's end lies on its plane; every other end is square.
    let joints: BTreeMap<EdgeId, crate::plate::Joint<'_>> = ends
        .iter()
        .map(|(edge, surface)| (*edge, crate::plate::Joint::Onto(surface)))
        .collect();
    let solid = plate(&sheet, &faces, t, Side::Out, &joints, tolerance)?;
    Ok(Part {
        plane: spec.plane,
        thickness: t,
        flats,
        bends,
        corners,
        bend_ends,
        cuts,
        sheet,
        edge_keys,
        sheet_faces,
        solid,
        marks,
        flanges: spec.flanges.len(),
    })
}

/// A bend that is not laid out yet: [`build`] fills every slot before
/// anything reads it (a flange's when its turn comes, a jog's when the
/// flat it crosses is laid out), and refuses the part otherwise.
const PLACEHOLDER_BEND: Bend = Bend {
    parent: 0,
    child: 0,
    angle: 0.0,
    radius: 0.0,
    k: 0.0,
    thickness: 0.0,
    allowance: 0.0,
    start: [0.0, 0.0],
    end: [0.0, 0.0],
    across: [0.0, 0.0],
    flat_length: 0.0,
    trim: [None, None],
};

/// A flat that is not laid out yet (see [`PLACEHOLDER_BEND`]).
fn empty_flat() -> Flat {
    Flat {
        outer: Vec::new(),
        holes: Vec::new(),
        transform: Transform::IDENTITY,
        tip: None,
        ends: Vec::new(),
        was: Vec::new(),
        seats: Vec::new(),
        ids: Vec::new(),
        hole_ids: Vec::new(),
        outlined: false,
        pieces: vec![0],
        added: Vec::new(),
        notches: Vec::new(),
    }
}

/// The bends in the order the folded sheet makes them: in the list's
/// order, each as soon as its parent flat is made (the base at once, a
/// flat when the bend it hangs from is). A part without jogs has every
/// parent before its child, so this is the list's order and its sheet is
/// made as before, bit for bit; a jog's far side comes after the flanges
/// on its edges in the list.
fn parents_first(bends: &[Bend]) -> Result<Vec<usize>, KernelError> {
    let mut made = vec![false; bends.len() + 1];
    if let Some(slot) = made.first_mut() {
        *slot = true;
    }
    let mut done = vec![false; bends.len()];
    let mut order = Vec::with_capacity(bends.len());
    // Each pass makes at least one bend or stops: at most one per bend.
    for _ in 0..=bends.len() {
        let mut any = false;
        for (i, b) in bends.iter().enumerate() {
            if done.get(i).copied().unwrap_or(true) || !made.get(b.parent).copied().unwrap_or(false)
            {
                continue;
            }
            order.push(i);
            if let Some(slot) = done.get_mut(i) {
                *slot = true;
            }
            if let Some(slot) = made.get_mut(b.child) {
                *slot = true;
            }
            any = true;
        }
        if !any {
            break;
        }
    }
    if order.len() != bends.len() {
        return Err(refuse(None, "a bend hangs from a flat no bend reaches"));
    }
    Ok(order)
}

/// The jogs of a part, laid out as the flats they cross are.
struct Jogs<'a> {
    spec: &'a Spec,
    marks: &'a [u64],
    nf: usize,
    thickness: f64,
    tolerance: &'a Tolerance,
}

impl Jogs<'_> {
    /// Lays out every jog still pending whose line's middle lies in a flat
    /// laid out (the near side keeps the flat's place, the web and the far
    /// side take theirs), in the jogs' order, until none is left that
    /// does: a jog across an earlier jog's far side follows it.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        flats: &mut [Flat],
        laid: &mut [bool],
        root: &mut [usize],
        bends: &mut [Bend],
        seated: &mut [(usize, Option<usize>, usize)],
        lifts: &mut [P2],
        pending: &mut [bool],
        jogged: &mut std::collections::BTreeSet<usize>,
    ) -> Result<(), KernelError> {
        let tol = self.tolerance.linear;
        // Each round lays out at least one jog or stops: at most one round
        // per jog.
        for _ in 0..=pending.len() {
            let mut any = false;
            for (g, jog) in self.spec.jogs.iter().enumerate() {
                if !pending.get(g).copied().unwrap_or(false) {
                    continue;
                }
                let first = self.nf + 2 * g;
                let [a, b] = jog.line;
                let middle = scale(add(a, b), 0.5);
                // The flats it may cross: laid out, not a web.
                let webs = |q: usize| q > self.nf && (q - self.nf) % 2 == 1;
                let holding: Vec<usize> = flats
                    .iter()
                    .enumerate()
                    .filter(|(q, flat)| {
                        laid.get(*q).copied().unwrap_or(false)
                            && !webs(*q)
                            && lifts
                                .get(*q)
                                .is_some_and(|&lift| jog::holds(flat, lift, middle, tol))
                    })
                    .map(|(q, _)| q)
                    .collect();
                let q = match holding.as_slice() {
                    [] => continue,
                    [q] => *q,
                    _ => {
                        return Err(refuse(
                            Some(first),
                            format!(
                                "jog {g}: the middle of its line lies in several flats as drawn (with every bend of no width): {holding:?}; draw it where one flat lies alone"
                            ),
                        ));
                    }
                };
                let unmarked = || refuse(Some(first), format!("jog {g} has no id"));
                let numbers = [
                    flange_number(self.marks, first).ok_or_else(unmarked)?,
                    flange_number(self.marks, first + 1).ok_or_else(unmarked)?,
                ];
                let flat = flats.get(q).ok_or_else(unmarked)?;
                let lift = lifts.get(q).copied().ok_or_else(unmarked)?;
                let made = jog::split(
                    g,
                    jog,
                    q,
                    flat,
                    lift,
                    first,
                    numbers,
                    self.thickness,
                    self.tolerance,
                )?;
                let from = root.get(q).copied().unwrap_or(q);
                let lost = || {
                    refuse(
                        Some(first),
                        format!("jog {g}: a slot of the part is missing"),
                    )
                };
                *flats.get_mut(q).ok_or_else(lost)? = made.near;
                *flats.get_mut(first + 1).ok_or_else(lost)? = made.web;
                *flats.get_mut(first + 2).ok_or_else(lost)? = made.far;
                let [b1, b2] = made.bends;
                *bends.get_mut(first).ok_or_else(lost)? = b1;
                *bends.get_mut(first + 1).ok_or_else(lost)? = b2;
                *seated.get_mut(first).ok_or_else(lost)? = (q, None, made.chord);
                *seated.get_mut(first + 1).ok_or_else(lost)? = (first + 1, None, 2);
                *lifts.get_mut(first + 1).ok_or_else(lost)? = made.web_lift;
                *lifts.get_mut(first + 2).ok_or_else(lost)? = made.far_lift;
                *laid.get_mut(first + 1).ok_or_else(lost)? = true;
                *laid.get_mut(first + 2).ok_or_else(lost)? = true;
                *root.get_mut(first + 2).ok_or_else(lost)? = from;
                *pending.get_mut(g).ok_or_else(lost)? = false;
                jogged.insert(q);
                jogged.insert(first + 2);
                any = true;
            }
            if !any {
                break;
            }
        }
        Ok(())
    }
}

/// A segment of an outline as the sketch curve the profile detection
/// reads.
fn sketch_curve(seg: &Segment) -> crate::profile::Curve2 {
    use crate::profile::Curve2;
    match &seg.shape {
        Shape::Line => Curve2::Line {
            start: seg.start,
            end: seg.end,
        },
        Shape::Arc { center, .. } => {
            let (start, end) = if seg.reversed {
                (seg.end, seg.start)
            } else {
                (seg.start, seg.end)
            };
            Curve2::Arc {
                center: *center,
                start,
                end,
            }
        }
        Shape::Circle { center, radius } => Curve2::Circle {
            center: *center,
            radius: *radius,
        },
        Shape::Spline(spline) => Curve2::Spline(spline.clone()),
    }
}

/// The flat pattern does not overlap itself: its outline (every flat's
/// and strip's edges that no strip continues, and the holes), run through
/// the profile detection of the sketch, is one region with all the holes
/// and the area the pieces add up to. Two pieces that cross, touch or lie
/// along each other anywhere but where the pattern joins them (a bend's
/// tangent lines, the corner point two strips share) make two curves of
/// the outline cross or touch away from a shared end, or more than two
/// ends meet at a point, which the detection names; a piece that lies
/// wholly inside another leaves the area short. Either fails the part,
/// naming the base or the flanges whose pieces meet; anything else the
/// detection refuses fails it too (fail closed).
fn pattern_clear(
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    corners: &[Corner],
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let chains = tagged_outline(flats, bends, cuts, tolerance.linear);
    let tagged: Vec<&(Segment, usize)> = chains.iter().flatten().collect();
    let curves: Vec<SketchCurve> = tagged
        .iter()
        .enumerate()
        .map(|(i, (seg, _))| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(seg),
        })
        .collect();
    let owner = |id: u64| {
        usize::try_from(id)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| tagged.get(i))
            .map(|(_, flat)| *flat)
    };
    let name = |flat: usize| match flat.checked_sub(1) {
        None => "the base".to_string(),
        Some(f) => format!("flange {f}"),
    };
    let meeting = |ids: &[u64]| {
        let mut owners: Vec<usize> = ids.iter().filter_map(|&id| owner(id)).collect();
        owners.sort_unstable();
        owners.dedup();
        let flange = owners.last().and_then(|f| f.checked_sub(1));
        let names: Vec<String> = owners.iter().map(|&f| name(f)).collect();
        let who = match names.as_slice() {
            [one] => format!("{one} runs into itself"),
            [a, b] => format!("{a} and {b} cross or touch"),
            _ => format!("{} cross or touch", names.join(", ")),
        };
        // Two flanges that meet at a closed corner: their extensions run
        // into the same corner of the pattern (found by the chassis's
        // range check, 2026-10-31: a mitre of walls of a thin sheet with
        // a small radius and K-factor).
        let closed = match owners.as_slice() {
            [a, b] if *a > 0 => corners.iter().find(|c| {
                c.closing.kind != CornerKind::Open
                    && (c.bends == [a - 1, b - 1] || c.bends == [b - 1, a - 1])
            }),
            _ => None,
        };
        let hint = closed.map_or_else(String::new, |c| {
            format!(
                "; they meet at a closed corner ({}), whose extensions run on past their bends into the same corner of the flat pattern: the extensions reach about the outside setback (radius + thickness) past the bends' ends, more than the bend allowance angle * (radius + kFactor * thickness) leaves between them at this thickness, radius and K-factor; give the corner a larger gap (each extension ends half of it short of the corner), or open it",
                c.closing.kind.name()
            )
        });
        refuse(
            flange,
            format!(
                "the flat pattern overlaps itself: {who} in the flat, so it cannot be cut from one sheet (shorten or move the flanges, or bend them from other edges){hint}"
            ),
        )
    };
    let profiles = match detect_profiles(&curves, tolerance) {
        Ok(profiles) => profiles,
        Err(KernelError::CrossingCurves { curves }) => return Err(meeting(&curves)),
        Err(KernelError::BranchingProfile { curves }) => return Err(meeting(&curves)),
        Err(other) => {
            return Err(refuse(
                None,
                format!(
                    "the flat pattern's outline could not be checked for overlapping itself: {other}"
                ),
            ));
        }
    };
    let holes: usize = flats.iter().map(|f| f.holes.len()).sum::<usize>() + cuts.len();
    let area = pattern_area(flats, bends, cuts);
    match profiles.as_slice() {
        [one]
            if one.holes.len() == holes
                && (one.area() - area).abs() <= tolerance.linear * area.abs().sqrt() * 8.0 =>
        {
            Ok(())
        }
        _ => Err(refuse(
            None,
            format!(
                "the flat pattern overlaps itself: its outline bounds {} region(s) where its pieces make one of area {area}, so a piece lies on another in the flat",
                profiles.len()
            ),
        )),
    }
}

/// `ids` as a message names the curves of a profile.
fn curves_named(ids: &[u64]) -> String {
    let list: Vec<String> = ids.iter().map(u64::to_string).collect();
    match list.as_slice() {
        [one] => format!("the closed profile of curve {one}"),
        _ => format!("the closed profile of curves {}", list.join(", ")),
    }
}

/// Every cut-out becomes a hole of the one flat it lies wholly inside in
/// the pattern: the flat's outline, its holes so far and the cut-out, run
/// through the profile detection of the sketch, are one profile with one
/// hole more and the flat's outer area (the test `reliefs_fit` makes).
/// Tried on every flat whose box meets the cut-out's, in the pattern's
/// order; one that fits none (across a bend's strip, past or touching the
/// pattern's edge, in a relief's notch or a hole of the base) is refused,
/// as is a cut-out with a profile inside it (a loose piece). The holes
/// then go through [`pattern_clear`] with the whole pattern.
fn place_cutouts(
    flats: &mut [Flat],
    cutouts: &[Cutout],
    done: &[bool],
    tolerance: &Tolerance,
) -> Result<Vec<Cut>, KernelError> {
    if cutouts.len() > MAX_CUTOUTS {
        return Err(refuse(
            None,
            format!("a part takes at most {MAX_CUTOUTS} holes and cut-outs beside its base"),
        ));
    }
    let tol = tolerance.linear;
    let mut across = Vec::new();
    for (c, Cutout { profile: cut, from }) in cutouts.iter().enumerate() {
        if done.get(c).copied().unwrap_or(false) {
            continue;
        }
        let named = format!("{} of {from}", curves_named(&cut.outer.sources(8)));
        if !cut.holes.is_empty() {
            return Err(refuse(
                None,
                format!(
                    "{named} has a closed profile inside it, which would be a loose piece of the sheet; a hole or cut-out is one closed profile"
                ),
            ));
        }
        // A hole runs clockwise, as the base's do.
        let hole = cut.outer.flipped().segments;
        let cb = cut.outer.bbox();
        let mut placed = false;
        for flat in flats.iter_mut() {
            let fb = crate::profile::Loop2 {
                segments: flat.outer.clone(),
            }
            .bbox();
            let apart = cb[0] > fb[2] + tol
                || cb[2] < fb[0] - tol
                || cb[1] > fb[3] + tol
                || cb[3] < fb[1] - tol;
            if apart {
                continue;
            }
            if hole_fits(&flat.outer, &flat.holes, &hole, tolerance) {
                flat.holes.push(hole.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            across.push(Cut {
                segments: hole,
                named,
            });
        }
    }
    Ok(across)
}

/// The edge of cut-out `c` that flange `i` sits on (`On::Cutout`): the
/// place of the curve's one segment in the cut-out's loop as a hole runs
/// (clockwise, the flat on its left). Refused with the reason: no such
/// cut-out, a cut-out with a profile inside it, a curve that is not one
/// straight segment of it.
fn cutout_edge(spec: &Spec, i: usize, c: usize, curve: u64) -> Result<usize, KernelError> {
    let Some(cut) = spec.cutouts.get(c) else {
        return Err(refuse(
            Some(i),
            format!(
                "it sits on an edge of cut-out {c}, and the part has {} cut-outs",
                spec.cutouts.len()
            ),
        ));
    };
    if !cut.profile.holes.is_empty() {
        return Err(refuse(
            Some(i),
            format!(
                "the cut-out of {} it sits on has a closed profile inside it, which would be a loose piece of the sheet",
                cut.from
            ),
        ));
    }
    let hole = cut.profile.outer.flipped().segments;
    let found: Vec<usize> = hole
        .iter()
        .enumerate()
        .filter(|(_, s)| s.source == curve)
        .map(|(k, _)| k)
        .collect();
    let k = match found.as_slice() {
        [k] => *k,
        [] => {
            return Err(refuse(
                Some(i),
                format!(
                    "the curve {curve} is no edge of the cut-out of {}",
                    cut.from
                ),
            ));
        }
        _ => {
            return Err(refuse(
                Some(i),
                format!(
                    "the curve {curve} is cut into several edges of the cut-out of {}; a flange needs one straight edge",
                    cut.from
                ),
            ));
        }
    };
    if !matches!(hole.get(k).map(|s| &s.shape), Some(Shape::Line)) {
        return Err(refuse(
            Some(i),
            format!(
                "the curve {curve} of the cut-out of {} is not straight; a flange sits on a straight edge (on a curved edge it would be a stretched or shrunk flange, not a bend: not supported)",
                cut.from
            ),
        ));
    }
    Ok(k)
}

/// When flat `f`, just laid out, holds a cut-out a flange sits on, places
/// in it every cut-out not placed yet that lies wholly inside it ([`hole_fits`], as [`place_cutouts`] does after
/// the corners are closed), in the cut-outs' order. A cut-out a flange
/// sits on gets the names its segments have without the flange (by the
/// hole's place in the flat, `[13, flat, hole, segment]`), so the pieces
/// its edge is split into are named from them and no other name changes.
fn place_early(
    flats: &mut [Flat],
    f: usize,
    spec: &Spec,
    carrying: &std::collections::BTreeSet<usize>,
    marks: &[u64],
    placed: &mut [Option<(usize, usize)>],
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    if spec.cutouts.len() > MAX_CUTOUTS {
        return Err(refuse(
            None,
            format!("a part takes at most {MAX_CUTOUTS} holes and cut-outs beside its base"),
        ));
    }
    let tol = tolerance.linear;
    let number = flat_number(marks, f).ok_or_else(|| refuse(f.checked_sub(1), "it has no id"))?;
    let Some(flat) = flats.get_mut(f) else {
        return Err(refuse(f.checked_sub(1), "a flat is missing"));
    };
    let fb = crate::profile::Loop2 {
        segments: flat.outer.clone(),
    }
    .bbox();
    let apart = |cb: [f64; 4]| {
        cb[0] > fb[2] + tol || cb[2] < fb[0] - tol || cb[1] > fb[3] + tol || cb[3] < fb[1] - tol
    };
    // Only a flat that holds a cut-out a flange sits on takes its cut-outs
    // now; every other flat takes them after the corners are closed, as a
    // part without such a flange does, so a louvre added in one flat
    // renumbers no hole of another.
    let carries = carrying.iter().any(|&c| {
        spec.cutouts.get(c).is_some_and(|cutout| {
            placed.get(c).copied().flatten().is_none()
                && cutout.profile.holes.is_empty()
                && !apart(cutout.profile.outer.bbox())
                && hole_fits(
                    &flat.outer,
                    &flat.holes,
                    &cutout.profile.outer.flipped().segments,
                    tolerance,
                )
        })
    });
    if !carries {
        return Ok(());
    }
    for (c, cutout) in spec.cutouts.iter().enumerate() {
        if placed.get(c).copied().flatten().is_some() || !cutout.profile.holes.is_empty() {
            continue;
        }
        if apart(cutout.profile.outer.bbox()) {
            continue;
        }
        let hole = cutout.profile.outer.flipped().segments;
        if !hole_fits(&flat.outer, &flat.holes, &hole, tolerance) {
            continue;
        }
        let h = flat.holes.len();
        flat.holes.push(hole);
        if carrying.contains(&c) {
            // The names its segments, and every hole's before it, have
            // without a flange on it (`edge_numbers`: by the hole's place),
            // which the pieces of the split edge go on from.
            while flat.hole_ids.len() <= h {
                let at = flat.hole_ids.len();
                let len = flat.holes.get(at).map_or(0, Vec::len);
                flat.hole_ids.push(
                    (0..len)
                        .map(|j| [13, number, at as u64, j as u64])
                        .collect(),
                );
            }
        }
        if let Some(slot) = placed.get_mut(c) {
            *slot = Some((f, h));
        }
    }
    Ok(())
}

/// The cut-outs that lie in no one flat are holes across bends when they
/// lie wholly inside the flat pattern, touching none of its edges, holes
/// or earlier cuts: the pattern's outline (as [`pattern_clear`] checks it),
/// with them, through the profile detection of the sketch, is one profile
/// with one hole more and the area less by the cut's (the test
/// [`hole_fits`] makes on one flat). One that does not (past or touching
/// the pattern's edge, in a relief's notch, in or touching a hole) is
/// refused naming its curves. How it crosses the tangent lines is checked
/// when the sheet is folded ([`cuts`]).
fn place_cuts(
    flats: &[Flat],
    bends: &[Bend],
    across: &[Cut],
    tolerance: &Tolerance,
) -> Result<Vec<Cut>, KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    if !across.is_empty()
        && let Some(i) = bends
            .iter()
            .position(|b| b.trim.iter().any(Option::is_some))
    {
        return Err(refuse(
            Some(i),
            "a hole across a bend in a part whose corner trims a bend is not supported (owed: the cells of a strip whose end is the plane's curve): open that corner or give it a relief",
        ));
    }
    let mut cuts: Vec<Cut> = Vec::with_capacity(across.len());
    for cut in across {
        let mut with = cuts.clone();
        with.push(cut.clone());
        let chains = tagged_outline(flats, bends, &with, tolerance.linear);
        let curves: Vec<SketchCurve> = chains
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, (seg, _))| SketchCurve {
                id: i as u64 + 1,
                curve: sketch_curve(seg),
            })
            .collect();
        let holes: usize = flats.iter().map(|f| f.holes.len()).sum::<usize>() + with.len();
        let area = pattern_area(flats, bends, &with);
        let fits = match detect_profiles(&curves, tolerance).as_deref() {
            Ok([one]) => {
                one.holes.len() == holes
                    && (one.area() - area).abs() <= tolerance.linear * area.abs().sqrt() * 8.0
            }
            _ => false,
        };
        if !fits {
            return Err(refuse(
                None,
                format!(
                    "{} lies wholly inside no one flat of the flat pattern (the base or a flange's flat) nor inside the pattern clear of its edges: a hole or cut-out past or touching the pattern's edge, in a relief's notch, or in or touching another hole is not supported",
                    cut.named
                ),
            ));
        }
        cuts.push(cut.clone());
    }
    Ok(cuts)
}

/// Whether `hole` lies wholly inside the flat of outline `outer` with the
/// holes `holes`, touching none of them (see [`place_cutouts`]).
fn hole_fits(
    outer: &[Segment],
    holes: &[Vec<Segment>],
    hole: &[Segment],
    tolerance: &Tolerance,
) -> bool {
    use crate::profile::{SketchCurve, detect_profiles};
    let curves: Vec<SketchCurve> = outer
        .iter()
        .chain(holes.iter().flatten())
        .chain(hole)
        .enumerate()
        .map(|(i, seg)| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(seg),
        })
        .collect();
    let Ok(profiles) = detect_profiles(&curves, tolerance) else {
        return false;
    };
    let area = crate::profile::Loop2 {
        segments: outer.to_vec(),
    }
    .signed_area();
    match profiles.as_slice() {
        [one] => {
            one.holes.len() == holes.len() + 1
                && (one.outer.signed_area() - area).abs()
                    <= tolerance.linear * area.abs().sqrt() * 8.0
        }
        _ => false,
    }
}

/// The base's outline with its reliefs is still one region with the same
/// holes: the profile detection of the sketch, run on its curves, finds one
/// profile whose outer loop has the outline's area (a notch that crosses
/// another edge or a hole, or a tear relief whose circle grows past the
/// base at a corner near 180 degrees, does not).
fn reliefs_fit(
    outer: &[Segment],
    holes: &[Vec<Segment>],
    relief: Relief,
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let curves: Vec<SketchCurve> = outer
        .iter()
        .chain(holes.iter().flatten())
        .enumerate()
        .map(|(i, seg)| SketchCurve {
            id: i as u64 + 1,
            curve: sketch_curve(seg),
        })
        .collect();
    let refused = || {
        refuse(
            None,
            format!(
                "the {} reliefs of size {} cut across the base's outline or a hole of it (a tear relief's circle, of radius size × tan(corner/2), grows without bound as a corner nears 180 degrees): make them smaller",
                relief.kind.name(),
                relief.size
            ),
        )
    };
    let profiles = detect_profiles(&curves, tolerance).map_err(|_| refused())?;
    let area = crate::profile::Loop2 {
        segments: outer.to_vec(),
    }
    .signed_area();
    match profiles.as_slice() {
        [one]
            if one.holes.len() == holes.len()
                && (one.outer.signed_area() - area).abs()
                    <= tolerance.linear * area.abs().sqrt() * 8.0 =>
        {
            Ok(())
        }
        _ => Err(refused()),
    }
}

/// Every flat a bend relief's notch is cut into keeps one closed outline
/// that crosses and touches nothing of itself, with its holes and its
/// outline's area: the flat's outer loop and holes, run through the
/// profile detection of the sketch, are one profile (as `reliefs_fit`
/// holds the base's corner reliefs). A notch reaches `r` along the edge
/// and into the flat for rect and round, and `2r` both ways for tear (the
/// disc of radius `r` tangent to the edge and to the strip's end): on a
/// short flat (a wall whose flat is short beside a large bend radius) or
/// beside a closed corner's short extension the disc runs out of the flat
/// across its other edges, which would make a flat that crosses itself
/// and an invalid body, so it is refused here, naming the flange whose
/// bend's notch it is (docs/design/sheetmetal.md › *Bend reliefs*).
fn bend_reliefs_fit(
    flats: &[Flat],
    bends: &[Bend],
    bend_ends: &[BendEnd],
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    use crate::profile::{SketchCurve, detect_profiles};
    let mut seen: Vec<usize> = Vec::new();
    for e in bend_ends {
        let Some(relief) = e.relief else { continue };
        let Some(f) = bends.get(e.bend).map(|b| b.parent) else {
            continue;
        };
        if seen.contains(&f) {
            continue;
        }
        seen.push(f);
        let Some(flat) = flats.get(f) else { continue };
        let segs: Vec<&Segment> = flat
            .outer
            .iter()
            .chain(flat.holes.iter().flatten())
            .collect();
        let curves: Vec<SketchCurve> = segs
            .iter()
            .enumerate()
            .map(|(i, seg)| SketchCurve {
                id: i as u64 + 1,
                curve: sketch_curve(seg),
            })
            .collect();
        // The bend end with a relief in this flat nearest the curves the
        // detection names (the first end when it names none).
        let blame = |ids: &[u64]| -> &BendEnd {
            let mine = || {
                bend_ends.iter().filter(|x| {
                    x.relief.is_some() && bends.get(x.bend).map(|b| b.parent) == Some(f)
                })
            };
            let at: Vec<P2> = ids
                .iter()
                .filter_map(|&id| {
                    let i = usize::try_from(id).ok()?.checked_sub(1)?;
                    segs.get(i).map(|s| s.start)
                })
                .collect();
            let near = |x: &BendEnd| {
                at.iter()
                    .map(|&p| distance(p, x.point))
                    .fold(f64::INFINITY, f64::min)
            };
            mine()
                .min_by(|a, b| near(a).total_cmp(&near(b)))
                .unwrap_or(e)
        };
        let refused = |ids: &[u64]| {
            let x = blame(ids);
            let reach = match relief.kind {
                ReliefKind::Tear => format!(
                    "a tear relief's circle, of radius {0}, reaches {1} along the edge and {1} into the flat",
                    relief.size,
                    2.0 * relief.size
                ),
                _ => format!(
                    "a {} relief reaches {} along the edge and into the flat",
                    relief.kind.name(),
                    relief.size
                ),
            };
            let parent = match f.checked_sub(1) {
                None => "the base".to_string(),
                Some(p) => format!("flange {p}"),
            };
            refuse(
                Some(x.bend),
                format!(
                    "its bend relief at its {} cuts across another edge of {parent}'s flat ({reach}), so the flat would cross itself: make the bend relief smaller, use a smaller shape (round or rect reach half as far as tear), or inset the flange further from the flat's other edges",
                    if x.end == 0 { "start" } else { "end" },
                ),
            )
        };
        let profiles = match detect_profiles(&curves, tolerance) {
            Ok(p) => p,
            Err(KernelError::CrossingCurves { curves }) => return Err(refused(&curves)),
            Err(KernelError::BranchingProfile { curves }) => return Err(refused(&curves)),
            Err(other) => {
                return Err(refuse(
                    Some(e.bend),
                    format!(
                        "its bend relief's flat ({}) could not be checked for crossing itself: {other}",
                        match f.checked_sub(1) {
                            None => "the base".to_string(),
                            Some(p) => format!("flange {p}'s"),
                        }
                    ),
                ));
            }
        };
        let area = crate::profile::Loop2 {
            segments: flat.outer.clone(),
        }
        .signed_area();
        match profiles.as_slice() {
            [one]
                if one.holes.len() == flat.holes.len()
                    && (one.outer.signed_area() - area).abs()
                        <= tolerance.linear * area.abs().sqrt() * 8.0 => {}
            _ => return Err(refused(&[])),
        }
    }
    Ok(())
}

fn check_flange(i: usize, f: &FlangeSpec, tol: f64) -> Result<(), KernelError> {
    if let Some(x) = f.inset.iter().find(|x| !(x.is_finite() && **x >= 0.0)) {
        return Err(refuse(
            Some(i),
            format!("a flange's inset is a distance from 0 up, got {x}"),
        ));
    }
    if f.outline.is_none() && !(f.length.is_finite() && f.length > tol) {
        return Err(refuse(
            Some(i),
            format!(
                "a flange's length has to be more than the tolerance, got {}",
                f.length
            ),
        ));
    }
    if !(f.angle.is_finite() && f.angle > 0.0 && f.angle <= PI) {
        return Err(refuse(
            Some(i),
            format!(
                "a bend's angle is more than 0 and at most 180 degrees, got {}",
                f.angle.to_degrees()
            ),
        ));
    }
    if !(f.radius.is_finite() && f.radius >= 0.0) {
        return Err(refuse(
            Some(i),
            format!(
                "a bend's inner radius cannot be below zero, got {}",
                f.radius
            ),
        ));
    }
    if !(f.radius > tol) {
        return Err(refuse(
            Some(i),
            "a sharp bend (an inner radius within the tolerance of zero) is not supported: its inner skin would be an edge and its outer a cylinder of radius T over the angle, which the one offset (the plate of a sheet tangent across its bends) cannot make, and a bend at some smallest radius in its place would be a part other than the one asked for; give the inner radius the tooling makes (in air bending about a sixth of the die opening)",
        ));
    }
    if !(f.k.is_finite() && (0.0..=1.0).contains(&f.k)) {
        return Err(refuse(
            Some(i),
            format!(
                "a K-factor is the neutral fibre's place across the thickness, from 0 (the inner surface) to 1 (the outer), got {}",
                f.k
            ),
        ));
    }
    Ok(())
}

/// A closed corner's gap is more than the tolerance (flats that touch
/// would make the part run through itself), and a butt's less than the
/// thickness (its covering flange would end behind the other's inside
/// face, covering nothing).
fn check_closing(what: &str, c: &Closing, t: f64, tol: f64) -> Result<(), KernelError> {
    if c.kind == CornerKind::Open {
        return Ok(());
    }
    if !(c.gap.is_finite() && c.gap > tol) {
        return Err(refuse(
            None,
            format!(
                "{what}: a closed corner's gap has to be more than the tolerance {tol}, got {}: flats that touch would make the part one solid through itself",
                c.gap
            ),
        ));
    }
    if c.kind == CornerKind::Butt && !(c.gap < t - tol) {
        return Err(refuse(
            None,
            format!(
                "{what}: a butt corner's gap ({}) has to be less than the thickness {t}: the covering flange would end behind the other's inside face and cover nothing (use overlap, or a smaller gap)",
                c.gap
            ),
        ));
    }
    Ok(())
}

/// The treatment of the corner where the flanges `bends` meet (the one on
/// the arriving edge first): the corner's own, else the part's; `over`
/// resolved to a flange of the two.
fn closing_of(spec: &Spec, bends: [usize; 2]) -> Result<Closing, KernelError> {
    let [fa, fb] = bends;
    let mut own = spec
        .corners
        .iter()
        .filter(|c| c.flanges == [fa, fb] || c.flanges == [fb, fa]);
    let closing = match (own.next(), own.next()) {
        (Some(_), Some(_)) => {
            return Err(refuse(
                None,
                format!("corners: the corner of flanges {fa} and {fb} is listed twice"),
            ));
        }
        (Some(c), None) => c.closing,
        (None, _) => spec.corner,
    };
    let over = match closing.kind {
        CornerKind::Open | CornerKind::Miter => None,
        CornerKind::Butt | CornerKind::Overlap => match closing.over {
            None => Some(fa),
            Some(o) if o == fa || o == fb => Some(o),
            Some(o) => {
                return Err(refuse(
                    None,
                    format!(
                        "corners: flange {o} is not one of the corner's flanges {fa} and {fb}, so it cannot cover the corner"
                    ),
                ));
            }
        },
    };
    Ok(Closing { over, ..closing })
}

/// A wall of a closed corner: the flat of flange `x` as folded, in the
/// pattern's axes.
struct Wall {
    /// A point of its inner skin (the bend's inside: the top for a bend
    /// up) and of its outer skin.
    inner: Vec3,
    outer: Vec3,
    /// Unit, square to the flat, from the outer skin to the inner one.
    inward: Vec3,
    /// The middle of the flat, half way through.
    middle: Vec3,
}

fn wall(flats: &[Flat], bends: &[Bend], x: usize, t: f64) -> Result<Wall, KernelError> {
    let (Some(b), Some(flat)) = (bends.get(x), flats.get(x + 1)) else {
        return Err(refuse(Some(x), "a corner's flange is missing"));
    };
    let up = b.angle > 0.0;
    let tr = flat.transform;
    let n = tr
        .vector(Vec3::Z)
        .normalized()
        .ok_or_else(|| refuse(Some(x), "a flange's plane is not finite"))?;
    let (a0, _) = b.far_line();
    let at = |z: f64| tr.point(Vec3::new(a0[0], a0[1], z));
    let (b0, b1) = tip(b);
    let mid = scale(add(add(a0, b.far_line().1), add(b0, b1)), 0.25);
    Ok(Wall {
        inner: at(if up { t } else { 0.0 }),
        outer: at(if up { 0.0 } else { t }),
        inward: if up { n } else { -n },
        middle: tr.point(Vec3::new(mid[0], mid[1], 0.5 * t)),
    })
}

/// Where a closed corner's end plane meets a flange's flat on its bottom
/// skin: how far past the tangent line's end (`near`) and the tip's end
/// (`far`) along the flange's edge outwards (negative: short of it, a
/// trim; 0 within the tolerance), those points, and the bend's trim.
struct Reach {
    near: f64,
    far: f64,
    near_point: P2,
    far_point: P2,
    trim: Option<Trim>,
}

/// Lays out the extensions of the flats at the closed corners: each runs
/// on sideways from its bend's end, along its tangent line and its tip,
/// to the line where its end plane (folded) meets the flat's bottom skin
/// (docs/design/sheetmetal.md › *Closed corners*). The planes, for the
/// covering flange `C` and the other `O` of a butt or overlap, or the walls
/// `A`, `B` of a mitre, `g` the gap:
///
/// - overlap: `C` to `O`'s outer skin; `O` to `C`'s inner skin moved `g`
///   inward;
/// - butt: `C` to `O`'s outer skin moved `g` inward; `O` as for overlap;
/// - miter: each to the plane through the line where the outer skins meet
///   with the normal `out_A − out_B` (the points as far from both outer
///   skins), moved `g/2` towards its own wall.
#[allow(clippy::too_many_arguments)]
fn close_corners(
    flats: &mut [Flat],
    bends: &mut [Bend],
    corners: &[Corner],
    thickness: f64,
    bend_relief: Option<Relief>,
    bend_ends: &mut Vec<BendEnd>,
    early: &[usize],
    tolerance: &Tolerance,
) -> Result<bool, KernelError> {
    let mut moved_any = false;
    // Per flange: the plane its extension ends on at its start and at its
    // end (the corner at its edge's start or end).
    let mut at_start: BTreeMap<usize, (Vec3, Vec3)> = BTreeMap::new();
    let mut at_end: BTreeMap<usize, (Vec3, Vec3)> = BTreeMap::new();
    // Per flange end at a mitre: the halving plane before its shift by
    // half the gap (a point of it and its normal), what decides whether
    // the bend's root may reach past its end plane (`trim::untrimmed_cut`).
    let mut halving: BTreeMap<(usize, usize), (Vec3, Vec3)> = BTreeMap::new();
    // Per flange end at a closed corner, the corner's other flange.
    let mut other_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    // The bends whose end plane cuts into them where they are not
    // trimmed: decided once every extension is laid out (below).
    let mut pending: Vec<(usize, usize, &'static str, f64, f64)> = Vec::new();
    for c in corners {
        let [fa, fb] = c.bends;
        let g = c.closing.gap;
        let planes: [(usize, (Vec3, Vec3)); 2] = match c.closing.kind {
            CornerKind::Open => continue,
            kind => {
                let (Some(ba), Some(bb)) = (bends.get(fa), bends.get(fb)) else {
                    return Err(refuse(Some(fb), "a corner's flange is missing"));
                };
                if (ba.angle > 0.0) != (bb.angle > 0.0) {
                    return Err(refuse(
                        Some(fb),
                        format!(
                            "the corner of flanges {fa} and {fb} cannot be closed: one is bent up and the other down, so their walls do not meet"
                        ),
                    ));
                }
                let (wa, wb) = (
                    wall(flats, bends, fa, thickness)?,
                    wall(flats, bends, fb, thickness)?,
                );
                match kind {
                    CornerKind::Miter => {
                        let (oa, ob) = (-wa.inward, -wb.inward);
                        let cos = oa.dot(ob);
                        let sin2 = 1.0 - cos * cos;
                        if !(sin2.sqrt() > tolerance.angular) {
                            return Err(refuse(
                                Some(fb),
                                format!(
                                    "the corner of flanges {fa} and {fb} cannot be mitred: their walls are parallel"
                                ),
                            ));
                        }
                        // A point on both outer skins: α oa + β ob.
                        let (da, db) = (oa.dot(wa.outer), ob.dot(wb.outer));
                        let alpha = (da - cos * db) / sin2;
                        let beta = (db - cos * da) / sin2;
                        let q = oa * alpha + ob * beta;
                        let n = (oa - ob)
                            .normalized()
                            .ok_or_else(|| refuse(Some(fb), "a mitre's plane has no normal"))?;
                        let (sa, sb) = (n.dot(wa.middle - q), n.dot(wb.middle - q));
                        if !(sa * sb < 0.0) {
                            return Err(refuse(
                                Some(fb),
                                format!(
                                    "the corner of flanges {fa} and {fb} cannot be mitred: both walls lie on one side of the plane that halves the angle between them"
                                ),
                            ));
                        }
                        let toward = |side: f64| q + n * (0.5 * g * side.signum());
                        // Flange fa's end is at the corner, fb's start.
                        halving.insert((fa, 1), (q, n));
                        halving.insert((fb, 0), (q, n));
                        [(fa, (toward(sa), n)), (fb, (toward(sb), n))]
                    }
                    _ => {
                        let cover = c.closing.over.unwrap_or(fa);
                        let (wc, wo, other) = if cover == fa {
                            (&wa, &wb, fb)
                        } else {
                            (&wb, &wa, fa)
                        };
                        let shift = if kind == CornerKind::Butt { g } else { 0.0 };
                        [
                            (cover, (wo.outer + wo.inward * shift, wo.inward)),
                            (other, (wc.inner + wc.inward * g, wc.inward)),
                        ]
                    }
                }
            }
        };
        other_of.insert((fa, 1), fb);
        other_of.insert((fb, 0), fa);
        for (x, plane) in planes {
            // Flange fa's edge arrives at the corner (its end), fb's leaves
            // it (its start).
            let side = if x == fa { &mut at_end } else { &mut at_start };
            side.insert(x, plane);
        }
    }
    let flanges: Vec<usize> = at_start.keys().chain(at_end.keys()).copied().collect();
    let mut done: Vec<usize> = Vec::new();
    for x in flanges {
        if done.contains(&x) {
            continue;
        }
        done.push(x);
        let b_copy = *bends
            .get(x)
            .ok_or_else(|| refuse(Some(x), "a corner's flange is missing"))?;
        let b = &b_copy;
        let flat = flats
            .get(x + 1)
            .ok_or_else(|| refuse(Some(x), "a corner's flange is missing"))?;
        // An extension runs the rectangle of a flange on past its bend,
        // replacing the side at that end: a flange with an outline of its
        // own is not closed, nor a side that carries a flange (it would
        // be the corner's end). Its tip may carry flanges (inset, a
        // tab), which keep their pieces: the extensions add pieces before
        // and after them (docs/design/sheetmetal.md › *Closed corners*).
        let pieces = flat.pieces.clone();
        let range = |k: usize| -> Option<std::ops::Range<usize>> {
            Some(*pieces.get(k)?..*pieces.get(k + 1)?)
        };
        // The outline as laid out, segment by segment: its tangent line,
        // the side at the bend's start, what lies between (a rectangle's
        // tip, or an outline's own pieces), the side at its end. An
        // extension replaces a side at a closed end and runs its
        // neighbour on to the end (docs/design/sheetmetal.md › *Closed
        // corners*).
        let m = pieces.len().saturating_sub(1);
        let (Some(tangent), Some(first), Some(last)) =
            (range(0), range(1), m.checked_sub(1).and_then(range))
        else {
            return Err(refuse(Some(x), "a flange's outline is missing a piece"));
        };
        let tip_range = first.end..last.start;
        if tangent != (0..1) || m < 4 || tip_range.is_empty() {
            return Err(refuse(
                Some(x),
                "a closed corner replaces a flange's side at its bend's end and runs the outline's next piece on to the corner; this flange's outline has no piece between its two sides (a triangle), so its corner cannot be closed (not supported, owed): open it",
            ));
        }
        // A side that carries flanges: closed, the side moves to the
        // corner's end with its flanges (only when the end lies along it).
        let mut carried: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for (k, (closed, side)) in [
            (at_start.contains_key(&x), &first),
            (at_end.contains_key(&x), &last),
        ]
        .into_iter()
        .enumerate()
        {
            if !closed {
                continue;
            }
            if let Some(list) = carried.get_mut(k) {
                *list = flat
                    .seats
                    .iter()
                    .filter(|s| s.hole.is_none() && side.contains(&s.seg))
                    .map(|s| s.bend)
                    .collect();
            }
        }
        let d = unit(sub(b.end, b.start)).ok_or_else(|| refuse(Some(x), "a bend has no length"))?;
        let back = flat
            .transform
            .inverse()
            .ok_or_else(|| refuse(Some(x), "a flange's motion cannot be undone"))?;
        let parent_tf = flats
            .get(b.parent)
            .map(|f| f.transform)
            .ok_or_else(|| refuse(Some(x), "a flange's parent is missing"))?;
        let (a0, a1) = b.far_line();
        // The pieces next to the sides, which an extension runs on: their
        // ends at the sides and their directions away from them.
        let (Some(next_start), Some(next_end)) = (
            flat.outer.get(tip_range.start),
            flat.outer.get(tip_range.end.saturating_sub(1)),
        ) else {
            return Err(refuse(Some(x), "a flange's outline is missing a piece"));
        };
        let (b0, b1) = (next_start.start, next_end.end);
        let away = |seg: &Segment, from_end: bool| -> Result<P2, KernelError> {
            let dir = if from_end {
                sub(seg.end, seg.start)
            } else {
                sub(seg.start, seg.end)
            };
            match (&seg.shape, unit(dir)) {
                (Shape::Line, Some(e)) => Ok(e),
                _ => Err(refuse(
                    Some(x),
                    "a closed corner runs the outline's piece next to the side on to the corner; that piece is not straight (not supported, owed): open the corner",
                )),
            }
        };
        let e_start = if at_start.contains_key(&x) {
            away(next_start, false)?
        } else {
            scale(d, -1.0)
        };
        let e_end = if at_end.contains_key(&x) {
            away(next_end, true)?
        } else {
            d
        };
        // How far along `e` from `near` and `far` the plane meets the
        // flat's bottom skin: past the bend's end (an extension), within
        // the tolerance of it (the end as it is), or short of it (the flat
        // trimmed, and where that is at the tangent line, the bend too:
        // docs/design/sheetmetal.md › *Trimming a bend*).
        let reach = |plane: (Vec3, Vec3),
                     e: P2,
                     e_far: P2,
                     near: P2,
                     far: P2|
         -> Result<Reach, KernelError> {
            let (q, n) = (back.point(plane.0), back.vector(plane.1));
            let (den, den_far) = (n.dot(lift(e)), n.dot(lift(e_far)));
            let tilted = n.length() * tolerance.angular;
            if !(den.abs() > tilted && den_far.abs() > tilted) {
                return Err(refuse(
                    Some(x),
                    "a closed corner's end runs along the flange, so the flange cannot reach it",
                ));
            }
            let snap = |v: f64| if v.abs() <= tolerance.linear { 0.0 } else { v };
            let s_near = snap(n.dot(q - lift(near)) / den);
            let s_far = snap(n.dot(q - lift(far)) / den_far);
            if !(s_near.is_finite() && s_far.is_finite()) {
                return Err(refuse(Some(x), "a closed corner's end plane is not finite"));
            }
            Ok(Reach {
                near: s_near,
                far: s_far,
                near_point: add(near, scale(e, s_near)),
                far_point: add(far, scale(e_far, s_far)),
                trim: None,
            })
        };
        let mut start = match at_start.get(&x) {
            Some(&plane) => Some(reach(plane, scale(d, -1.0), e_start, a0, b0)?),
            None => None,
        };
        let mut end = match at_end.get(&x) {
            Some(&plane) => Some(reach(plane, d, e_end, a1, b1)?),
            None => None,
        };
        for (which, side, plane) in [
            (0usize, &mut start, at_start.get(&x)),
            (1usize, &mut end, at_end.get(&x)),
        ] {
            let (Some(r), Some(&plane)) = (side.as_mut(), plane) else {
                continue;
            };
            if r.near < 0.0 {
                let t = trim::trim(x, b, &parent_tf, which, plane, tolerance)?.ok_or_else(|| {
                    refuse(
                        Some(x),
                        "a closed corner's end plane cuts the flange's tangent line but not its bend's end there",
                    )
                })?;
                r.near_point = trim::far_point(b, &t);
                r.trim = Some(t);
            } else if let Some((skin, phi, depth)) = {
                // The root's own cut is allowed only at a mitre whose
                // halving plane passes through the corner's point on both
                // skins (a symmetric mitre): then a cut no deeper than the
                // root's keeps the bend on its side of that plane, which
                // the other wall's material does not cross. Anywhere else
                // nothing proves the other bend leaves room (review of
                // cc7b9228): every cut counts.
                let corner = if which == 0 { b.start } else { b.end };
                let root_allowed = halving.get(&(x, which)).is_some_and(|&(q, n)| {
                    [0.0, thickness].iter().all(|&z| {
                        let p = parent_tf.point(Vec3::new(corner[0], corner[1], z));
                        n.dot(p - q).abs() <= tolerance.linear
                    })
                });
                trim::untrimmed_cut(x, b, &parent_tf, which, plane, root_allowed, tolerance)?
            } {
                pending.push((x, which, skin, phi, depth));
            }
        }
        // A side that carries flanges moves to the end with them: the end
        // must lie along the side (the same reach at both its ends) and run
        // the flat on, not cut it back.
        let mut shift: [Option<P2>; 2] = [None, None];
        for (k, r) in [(0usize, &start), (1usize, &end)] {
            let (Some(r), Some(list)) = (r.as_ref(), carried.get(k)) else {
                continue;
            };
            if list.is_empty() {
                continue;
            }
            let (near, far) = if k == 0 { (a0, b0) } else { (a1, b1) };
            let v = sub(r.near_point, near);
            let w = sub(r.far_point, far);
            if !(distance(v, w) <= tolerance.linear && r.near >= 0.0 && r.far >= 0.0) {
                return Err(refuse(
                    Some(x),
                    format!(
                        "its corner at the {} of its bend is closed, which moves its side there to the corner's end, and that side carries flange {}: the end does not lie along the side (walls leaning or not square to each other) or cuts the flange back, so the flange cannot move with it (not supported): open that corner, make the walls square, or put the flange on the tip",
                        if k == 0 { "start" } else { "end" },
                        list.first().copied().unwrap_or(0)
                    ),
                ));
            }
            if let Some(slot) = shift.get_mut(k) {
                *slot = Some(v);
            }
        }
        // What is left of the tangent line and the tip between the two
        // ends must not be a sliver (or nothing).
        let floor = SLIVER_TOLERANCES * tolerance.linear;
        let along = |p: P2| dot(sub(p, a0), d);
        let near_lo = start.as_ref().map_or(along(a0), |r| along(r.near_point));
        let near_hi = end.as_ref().map_or(along(a1), |r| along(r.near_point));
        if !(near_hi - near_lo >= floor) {
            return Err(refuse(
                Some(x),
                format!(
                    "closing its corners trims the flange to nothing (or less than {floor}, {SLIVER_TOLERANCES} tolerances) along its bend or its tip: give the corners a relief or a gap, or open one"
                ),
            ));
        }
        let old = flat.outer.clone();
        // The tangent line, shortened where a trim cuts the bend's end.
        let cut_at = |r: &Option<Reach>, p: P2| match r {
            Some(r) if r.near <= 0.0 => r.near_point,
            _ => p,
        };
        let mut outer = vec![line(cut_at(&end, a1), cut_at(&start, a0))];
        let mut was = vec![Some(0)];
        // Per segment, the number a piece an extension adds is named by:
        // its place in the outline as if each side of the rectangle were
        // one segment, so flanges on the tip renumber none of them.
        let mut added: Vec<Option<usize>> = vec![None];
        let mut ends = Vec::new();
        let mut remap: BTreeMap<usize, usize> = BTreeMap::new();
        remap.insert(0, 0);
        let keep = |outer: &mut Vec<Segment>,
                    was: &mut Vec<Option<usize>>,
                    added: &mut Vec<Option<usize>>,
                    remap: &mut BTreeMap<usize, usize>,
                    r: std::ops::Range<usize>|
         -> Result<(), KernelError> {
            for j in r {
                let seg = old
                    .get(j)
                    .ok_or_else(|| refuse(Some(x), "a flange's outline is missing a piece"))?;
                remap.insert(j, outer.len());
                outer.push(seg.clone());
                was.push(Some(j));
                added.push(None);
            }
            Ok(())
        };
        // A side that carries flanges, moved by `v` to the corner's end
        // (the line `from → to`): its pieces keep their names and seats;
        // those that lie on the end and carry no bend end on its plane.
        let seated: Vec<usize> = flat
            .seats
            .iter()
            .filter(|s| s.hole.is_none())
            .map(|s| s.seg)
            .collect();
        let moved = |outer: &mut Vec<Segment>,
                     was: &mut Vec<Option<usize>>,
                     added: &mut Vec<Option<usize>>,
                     remap: &mut BTreeMap<usize, usize>,
                     ends: &mut Vec<(usize, Option<(Vec3, Vec3)>)>,
                     r: std::ops::Range<usize>,
                     v: P2,
                     on: (P2, P2, Option<(Vec3, Vec3)>)|
         -> Result<(), KernelError> {
            let (from, to, plane) = on;
            let line_dir = unit(sub(to, from));
            let on_end =
                |p: P2| line_dir.is_some_and(|e| cross(e, sub(p, from)).abs() <= tolerance.linear);
            for j in r {
                let seg = old
                    .get(j)
                    .ok_or_else(|| refuse(Some(x), "a flange's outline is missing a piece"))?;
                let seg = shifted(seg, v);
                if !seated.contains(&j) && on_end(seg.start) && on_end(seg.end) {
                    ends.push((outer.len(), plane));
                }
                remap.insert(j, outer.len());
                outer.push(seg);
                was.push(Some(j));
                added.push(None);
            }
            Ok(())
        };
        // A flange on the tip whose bend reaches an end the extension runs
        // on past now ends inside its edge.
        let touching = |at: usize| {
            flat.seats
                .iter()
                .find(|s| s.hole.is_none() && s.seg == at)
                .map(|s| s.bend)
        };
        let mut inside: Vec<(usize, usize, P2)> = Vec::new();
        // Per segment of the new outline, a piece of a bend relief's
        // notch cut into an extension (`Flat::notches`).
        let mut notched: Vec<(usize, [usize; 3])> = Vec::new();
        // The piece of the extension along the tip from its far end `far`
        // to the tip's end `p` (`which` 0, at the start) or from `p` to
        // `far` (1, at the end), in the outline's order: with a bend
        // relief at the end of tip flange `t` there, the plain piece ends
        // the relief's size short of `p` and the notch is cut between it
        // and the strip's end, as `split_at_seats` cuts one into an edge
        // (docs/design/sheetmetal.md › *Bend reliefs*). Each piece with
        // its place in the notch, `None` for the plain piece.
        let floor = SLIVER_TOLERANCES * tolerance.linear;
        let along_tip = |tf: Option<usize>,
                         which: usize,
                         p: P2,
                         far: P2,
                         plane: Option<(Vec3, Vec3)>|
         -> Result<Vec<(Segment, Option<usize>)>, KernelError> {
            let plain = |a: P2, b: P2| vec![(line(a, b), None)];
            let (Some(t), Some(relief)) = (tf, bend_relief) else {
                return Ok(if which == 0 {
                    plain(far, p)
                } else {
                    plain(p, far)
                });
            };
            let into = scale(
                bends
                    .get(t)
                    .ok_or_else(|| refuse(Some(t), "a bend is missing"))?
                    .across,
                -1.0,
            );
            // How far the extension runs on past `p` along the tip on the
            // bottom skin: to `far`. The notch must leave a sliver of it
            // there (the rest of the bottom skin, a slanted end across
            // the flat included, is held exactly by `bend_reliefs_fit`).
            let bottom = distance(far, p);
            let left = bottom - relief.size;
            if !(left >= floor) {
                return Err(refuse(
                    Some(t),
                    format!(
                        "its bend reaches the {} of flange {x}'s tip, which the closed corner there runs on past by {bottom}: its bend relief of size {} there leaves {left} of the extension beside it (at least {floor}, {SLIVER_TOLERANCES} tolerances, must stay); make the relief smaller, give the corner a smaller gap or inset the flange",
                        if which == 0 { "start" } else { "end" },
                        relief.size
                    ),
                ));
            }
            // On the top skin the end plane may lie nearer (a mitre's end
            // is slanted across the thickness, and any end may slant
            // across the flat): the least run-on there over the box the
            // notch lies in (`reach` along the tip and as deep: `r` for
            // rect and round, `2r` for tear's disc), the plane being
            // linear, at its two corners on the tip's side of the box.
            let reach = match relief.kind {
                ReliefKind::Tear => 2.0 * relief.size,
                _ => relief.size,
            };
            if let (Some((q, n)), Some(e)) = (plane, unit(sub(far, p))) {
                let (q, n) = (back.point(q), back.vector(n));
                let den = n.dot(lift(e));
                if den.abs() > n.length() * tolerance.angular {
                    let top = [0.0, reach]
                        .iter()
                        .map(|&depth| {
                            let at = add(p, scale(into, depth));
                            n.dot(q - Vec3::new(at[0], at[1], thickness)) / den
                        })
                        .fold(f64::INFINITY, f64::min);
                    let left = top - reach;
                    if !(left >= floor) {
                        return Err(refuse(
                            Some(t),
                            format!(
                                "its bend reaches the {} of flange {x}'s tip, which the closed corner there runs on past by {top} (the least on either skin, over the notch's depth: the corner's end plane slants across the thickness or the flat): its {} bend relief of size {} reaches {reach} along it, which leaves {left} of the extension beside it on the top skin (at least {floor}, {SLIVER_TOLERANCES} tolerances, must stay); make the relief smaller or rect or round, give the corner a smaller gap or inset the flange",
                                if which == 0 { "start" } else { "end" },
                                relief.kind.name(),
                                relief.size
                            ),
                        ));
                    }
                }
            }
            let deep = add(p, scale(into, relief.size));
            let numbered = |pieces: Vec<Segment>| {
                pieces
                    .into_iter()
                    .enumerate()
                    .map(|(j, seg)| (seg, Some(j)))
                    .collect::<Vec<_>>()
            };
            Ok(if which == 0 {
                let stop = add(p, scale(e_start, relief.size));
                let mut pieces = notch(p, e_start, into, relief)?;
                pieces.push(line(deep, p));
                let mut out = plain(far, stop);
                out.extend(numbered(pieces));
                out
            } else {
                let stop = add(p, scale(e_end, relief.size));
                let mut pieces = vec![line(p, deep)];
                pieces.extend(notch(p, into, e_end, relief)?);
                let mut out = numbered(pieces);
                out.extend(plain(stop, far));
                out
            })
        };
        let mut lay = |outer: &mut Vec<Segment>,
                       was: &mut Vec<Option<usize>>,
                       added: &mut Vec<Option<usize>>,
                       t: Option<usize>,
                       which: usize,
                       pieces: Vec<(Segment, Option<usize>)>,
                       number: usize| {
            for (seg, j) in pieces {
                match (t, j) {
                    (Some(t), Some(j)) => {
                        notched.push((outer.len(), [t, which, j]));
                        added.push(None);
                    }
                    _ => added.push(Some(number)),
                }
                outer.push(seg);
                was.push(None);
            }
        };
        match &start {
            Some(r) => {
                if r.near > 0.0 {
                    outer.push(line(a0, r.near_point));
                    was.push(None);
                    added.push(Some(1));
                }
                match shift.first().copied().flatten() {
                    Some(v) => moved(
                        &mut outer,
                        &mut was,
                        &mut added,
                        &mut remap,
                        &mut ends,
                        first.clone(),
                        v,
                        (r.near_point, r.far_point, at_start.get(&x).copied()),
                    )?,
                    None => {
                        ends.push((outer.len(), at_start.get(&x).copied()));
                        outer.push(line(r.near_point, r.far_point));
                        was.push(Some(first.start));
                        added.push(None);
                    }
                }
                let t = touching(tip_range.start);
                if r.far > 0.0 {
                    let pieces = along_tip(t, 0, b0, r.far_point, at_start.get(&x).copied())?;
                    lay(&mut outer, &mut was, &mut added, t, 0, pieces, 3);
                    if let Some(t) = t {
                        inside.push((t, 0, b0));
                    }
                }
            }
            None => keep(&mut outer, &mut was, &mut added, &mut remap, first.clone())?,
        }
        let tip_at = outer.len();
        // The places, as if each side were one segment, of the pieces the
        // end's extension adds: after the pieces between the sides.
        let tip_place = if start.is_some() { 4 } else { 2 } + m.saturating_sub(4);
        keep(
            &mut outer,
            &mut was,
            &mut added,
            &mut remap,
            tip_range.clone(),
        )?;
        // A trimmed tip: its first (last) piece starts (ends) where the
        // end meets it, which must be a plain piece of the tip that keeps
        // more than a sliver, carrying no flange.
        let tip_end = outer.len();
        for (which, r) in [(0usize, &start), (1usize, &end)] {
            let Some(r) = r.as_ref().filter(|r| r.far < 0.0) else {
                continue;
            };
            let (at, from) = if which == 0 {
                (tip_at, tip_range.start)
            } else {
                (tip_end.saturating_sub(1), tip_range.end.saturating_sub(1))
            };
            let label = if which == 0 { "start" } else { "end" };
            if let Some(t) = touching(from) {
                return Err(refuse(
                    Some(x),
                    format!(
                        "closing its corner at the {label} of its bend trims its tip back past flange {t}'s bend on it: open that corner or move the flange"
                    ),
                ));
            }
            let seg = outer
                .get_mut(at)
                .ok_or_else(|| refuse(Some(x), "a flange's tip is missing"))?;
            let dir = unit(sub(seg.end, seg.start)).unwrap_or(d);
            let left = if which == 0 {
                dot(sub(seg.end, r.far_point), dir)
            } else {
                dot(sub(r.far_point, seg.start), dir)
            };
            if !(matches!(seg.shape, Shape::Line) && left >= floor) {
                return Err(refuse(
                    Some(x),
                    format!(
                        "closing its corner at the {label} of its bend trims its tip back by {} past the end of its first piece (at least {floor} must stay): give the corner a relief or a gap, or open it",
                        -r.far
                    ),
                ));
            }
            if which == 0 {
                seg.start = r.far_point;
            } else {
                seg.end = r.far_point;
            }
        }
        match &end {
            Some(r) => {
                let t = touching(tip_range.end.saturating_sub(1));
                if r.far > 0.0 {
                    if let Some(t) = t {
                        inside.push((t, 1, b1));
                    }
                    let pieces = along_tip(t, 1, b1, r.far_point, at_end.get(&x).copied())?;
                    lay(
                        &mut outer,
                        &mut was,
                        &mut added,
                        t,
                        1,
                        pieces,
                        tip_place + 1,
                    );
                }
                match shift.get(1).copied().flatten() {
                    Some(v) => moved(
                        &mut outer,
                        &mut was,
                        &mut added,
                        &mut remap,
                        &mut ends,
                        last.clone(),
                        v,
                        (r.far_point, r.near_point, at_end.get(&x).copied()),
                    )?,
                    None => {
                        ends.push((outer.len(), at_end.get(&x).copied()));
                        outer.push(line(r.far_point, r.near_point));
                        was.push(Some(last.start));
                        added.push(None);
                    }
                }
                if r.near > 0.0 {
                    outer.push(line(r.near_point, a1));
                    was.push(None);
                    added.push(Some(tip_place + 3));
                }
            }
            None => keep(&mut outer, &mut was, &mut added, &mut remap, last.clone())?,
        }
        for &(t, which, point) in &inside {
            bend_ends.push(BendEnd {
                bend: t,
                end: which,
                point,
                relief: bend_relief,
            });
        }
        let mut notches: Vec<Option<[usize; 3]>> = Vec::new();
        if !notched.is_empty() {
            notches = vec![None; outer.len()];
            for (at, key) in notched {
                if let Some(slot) = notches.get_mut(at) {
                    *slot = Some(key);
                }
            }
        }
        let flat = flats
            .get_mut(x + 1)
            .ok_or_else(|| refuse(Some(x), "a corner's flange is missing"))?;
        flat.outer = outer;
        flat.was = was;
        flat.added = added;
        flat.notches = notches;
        flat.tip = flat.tip.map(|_| tip_at);
        // A bend on a hole's edge keeps its place: the holes are not
        // touched.
        for seat in flat.seats.iter_mut().filter(|s| s.hole.is_none()) {
            seat.seg = *remap
                .get(&seat.seg)
                .ok_or_else(|| refuse(Some(seat.bend), "a flange lost its edge"))?;
        }
        flat.ends = ends
            .into_iter()
            .filter_map(|(segment, plane)| {
                plane.map(|(point, normal)| End {
                    segment,
                    point,
                    normal,
                })
            })
            .collect();
        let trims = [
            start.as_ref().and_then(|r| r.trim),
            end.as_ref().and_then(|r| r.trim),
        ];
        if let Some(bend) = bends.get_mut(x) {
            bend.trim = trims;
        }
        // The flanges on a side moved to the corner's end move with it,
        // and every flange beyond them: their pattern by `v` (the fold
        // places them again from their bends).
        for (k, v) in shift.iter().enumerate() {
            let (Some(v), Some(list)) = (*v, carried.get(k)) else {
                continue;
            };
            let mut set: std::collections::BTreeSet<usize> = list
                .iter()
                .filter_map(|&i| bends.get(i).map(|b| b.child))
                .collect();
            for _ in 0..bends.len() {
                let more: Vec<usize> = bends
                    .iter()
                    .filter(|b| set.contains(&b.parent) && !set.contains(&b.child))
                    .map(|b| b.child)
                    .collect();
                if more.is_empty() {
                    break;
                }
                set.extend(more);
            }
            if let Some(f) = early.iter().find(|f| set.contains(f)) {
                return Err(refuse(
                    f.checked_sub(1),
                    format!(
                        "it carries a flange on a cut-out's edge and moves with flange {x}'s side to the closed corner's end, which would move the cut-out away from where it is drawn (not supported): open that corner"
                    ),
                ));
            }
            for &f in &set {
                if let Some(fl) = flats.get_mut(f) {
                    fl.outer = fl.outer.iter().map(|s| shifted(s, v)).collect();
                    fl.holes = fl
                        .holes
                        .iter()
                        .map(|h| h.iter().map(|s| shifted(s, v)).collect())
                        .collect();
                }
            }
            for (i, bend) in bends.iter_mut().enumerate() {
                if set.contains(&bend.child) {
                    bend.start = add(bend.start, v);
                    bend.end = add(bend.end, v);
                    for e in bend_ends.iter_mut().filter(|e| e.bend == i) {
                        e.point = add(e.point, v);
                    }
                }
            }
            moved_any = true;
        }
    }
    // A bend its end plane cuts into where it is not trimmed is kept when
    // the other flange's flat, extension and all, lies wholly past the
    // bend's square end: the bend's material lies on its own side of the
    // plane square to its tangent line through its end (it runs along the
    // line only that far), the other bend meets it there only at the
    // corner's point (square ends at a convex corner), so a flat beyond
    // that plane by more than the tolerance cannot meet it, whatever the
    // closing plane does. The flat's least along the line, over its folded
    // outline on both skins: lines at their ends, arcs and circles at their
    // ends and the one angle where the least lies, a spline over its
    // control points (a lower bound, safe). Otherwise refused: nothing
    // proves the cut is clear (review of cc7b9228).
    for (x, which, skin, phi, depth) in pending {
        let b = bends
            .get(x)
            .ok_or_else(|| refuse(Some(x), "a corner's flange is missing"))?;
        let clear = other_of.get(&(x, which)).is_some_and(|&o| {
            past_the_end(flats, b, which, o, thickness).is_some_and(|m| m > tolerance.linear)
        });
        if !clear {
            return Err(refuse(
                Some(x),
                format!(
                    "the closed corner at its bend's {} has its end plane cut into the bend's {skin} skin by {depth} at {} degrees from the parent's tangent line, where the flange itself is not cut back, and the other flange's flat comes back over the bend's end (a bend down whose outside is cut, a cut through the middle of the bend, a mitre of walls unlike each other): trimming such a bend is not supported (owed); give the corner a relief (or a larger one), mitre it, or open it",
                    if which == 0 { "start" } else { "end" },
                    phi.to_degrees()
                ),
            ));
        }
    }
    bend_ends.sort_by(|a, b| a.bend.cmp(&b.bend).then(a.end.cmp(&b.end)));
    Ok(moved_any)
}

/// How far flange `other`'s flat (as folded, its outline on both skins)
/// lies at least past bend `b`'s square end at `which` (0 its start, 1
/// its end), along the bend's tangent line away from the bend: lines at
/// their ends, arcs and circles also at the angle where the least lies, a
/// spline at its control points (a lower bound). `None` when a flat is
/// missing or a number is not finite.
fn past_the_end(flats: &[Flat], b: &Bend, which: usize, other: usize, t: f64) -> Option<f64> {
    let parent = flats.get(b.parent)?;
    let flat = flats.get(other + 1)?;
    let d = unit(sub(b.end, b.start))?;
    let (corner, sign) = if which == 0 {
        (b.start, -1.0)
    } else {
        (b.end, 1.0)
    };
    let dir = parent.transform.vector(lift(d)) * sign;
    let p = parent.transform.point(lift(corner));
    let tr = flat.transform;
    let f = |q: P2, z: f64| dir.dot(tr.point(Vec3::new(q[0], q[1], z)) - p);
    let mut least = f64::INFINITY;
    for z in [0.0, t] {
        // The gradient of `f` in the flat's own axes.
        let (g0, gx, gy) = (f([0.0, 0.0], z), f([1.0, 0.0], z), f([0.0, 1.0], z));
        let grad = [gx - g0, gy - g0];
        for seg in &flat.outer {
            least = least.min(f(seg.start, z)).min(f(seg.end, z));
            match &seg.shape {
                Shape::Line => {}
                &Shape::Arc { center, radius } | &Shape::Circle { center, radius } => {
                    let norm = grad[0].phypot(grad[1]);
                    if norm > 0.0 {
                        let at = [
                            center[0] - radius * grad[0] / norm,
                            center[1] - radius * grad[1] / norm,
                        ];
                        let inside = match seg.angles() {
                            Some((begin, sweep)) => {
                                let a = (at[1] - center[1]).patan2(at[0] - center[0]);
                                (a - begin).rem_euclid(std::f64::consts::TAU) <= sweep
                            }
                            None => true,
                        };
                        if inside {
                            least = least.min(f(at, z));
                        }
                    }
                }
                Shape::Spline(sp) => {
                    for &c in &sp.control {
                        least = least.min(f(c, z));
                    }
                }
            }
        }
    }
    least.is_finite().then_some(least)
}

/// The number the piece at place `seg` of flat `flat`'s outline that a
/// closed corner's extension adds is named by ([`Flat::added`]).
fn added_number(flats: &[Flat], flat: usize, seg: usize) -> usize {
    flats
        .get(flat)
        .and_then(|f| f.added.get(seg).copied().flatten())
        .unwrap_or(seg)
}

/// What the segment at place `seg` of flat `flat`'s outline is: its
/// place with every corner open, a piece a closed corner's extension
/// adds, or a piece of a bend relief's notch cut into an extension.
pub(crate) fn outline_key(flats: &[Flat], flat: usize, seg: usize) -> EdgeKey {
    let f = flats.get(flat);
    if let Some([bend, end, piece]) = f.and_then(|f| f.notches.get(seg).copied().flatten()) {
        return EdgeKey::Notch { bend, end, piece };
    }
    match f.and_then(|f| f.was.get(seg).copied().flatten()) {
        Some(open) => EdgeKey::Outline { flat, seg: open },
        None => EdgeKey::Piece {
            flat,
            seg: added_number(flats, flat, seg),
        },
    }
}

/// The seat of bend `i`: the edge of its parent flat it starts from.
fn seat_of(flats: &[Flat], i: usize, bend: &Bend) -> Result<Seat, KernelError> {
    flats
        .get(bend.parent)
        .and_then(|f| f.seats.iter().find(|s| s.bend == i))
        .copied()
        .ok_or_else(|| refuse(Some(i), "a flange lost its edge"))
}

/// Per flange, the number the pieces of a split edge name its bend by:
/// its id ([`Spec::ids`]), or its place plus 1 when none are given.
fn flange_marks(spec: &Spec) -> Result<Vec<u64>, KernelError> {
    let mut marks = flange_ids(spec)?;
    // The jogs' bends after the flanges', each named by its own id.
    let mut seen: std::collections::BTreeSet<u64> = marks.iter().copied().collect();
    let n = spec.flanges.len();
    for (g, jog) in spec.jogs.iter().enumerate() {
        for id in jog.ids {
            if id == 0 || id > MAX_FLANGE_ID || !seen.insert(id) {
                return Err(refuse(
                    Some(n + 2 * g),
                    format!("jog {g}: its id {id} is 0, too large or a flange's or another jog's"),
                ));
            }
            marks.push(id);
        }
    }
    Ok(marks)
}

fn flange_ids(spec: &Spec) -> Result<Vec<u64>, KernelError> {
    let n = spec.flanges.len();
    if spec.ids.is_empty() {
        return Ok((1..=n as u64).collect());
    }
    if spec.ids.len() != n {
        return Err(refuse(
            None,
            format!("{} flange ids for {n} flanges", spec.ids.len()),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, &id) in spec.ids.iter().enumerate() {
        if id == 0 || id > MAX_FLANGE_ID || !seen.insert(id) {
            return Err(refuse(
                Some(i),
                format!("its id {id} is 0, too large or another flange's"),
            ));
        }
    }
    Ok(spec.ids.clone())
}

/// The piece of the straight segment `seg` from `a` to `b` (both on it).
fn piece(seg: &Segment, a: P2, b: P2) -> Segment {
    Segment {
        start: a,
        end: b,
        ..seg.clone()
    }
}

/// Splits every edge flanges sit on where their bends start and end, so
/// each bend's tangent line on its parent's side is one segment of the
/// parent's loop, and records the seats (docs/design/sheetmetal.md ›
/// *Inset flanges*). Bends on one edge must keep more than the tolerance
/// apart (touching or overlapping, two strips would lie along each
/// other). An edge whose one flange spans it whole stays as it is.
///
/// Every end of a bend that lies inside its edge (the edge runs on past
/// it) is listed ([`BendEnd`]); with a bend relief of size `r`, the piece
/// of the edge beside it ends `r` short of it and a notch of the relief's
/// shape is cut into the parent there: [`notch`] at the tangent line's end
/// `p` between the edge running on (`±d`) and the parent's side (`−û`),
/// then the line from `p − r û` back to `p`, in line with the strip's end
/// (docs/design/sheetmetal.md › *Bend reliefs*). The pieces of the edge
/// keep their names; the notch's are `[11, b, end, j]` by the flange's
/// number.
fn split_at_seats(
    flats: &mut [Flat],
    bends: &[Bend],
    seated: &[(usize, Option<usize>, usize)],
    marks: &[u64],
    bend_relief: Option<Relief>,
    tol: f64,
) -> Result<Vec<BendEnd>, KernelError> {
    let mut bend_ends: Vec<BendEnd> = Vec::new();
    let mut groups: BTreeMap<(usize, Option<usize>, usize), Vec<usize>> = BTreeMap::new();
    for (i, &key) in seated.iter().enumerate() {
        groups.entry(key).or_default().push(i);
    }
    for (f, flat) in flats.iter_mut().enumerate() {
        let n_holes = flat.holes.len();
        for hole in std::iter::once(None).chain((0..n_holes).map(Some)) {
            let segs = match hole {
                None => &flat.outer,
                Some(h) => match flat.holes.get(h) {
                    Some(l) => l,
                    None => continue,
                },
            };
            if !groups.keys().any(|&(gf, gh, _)| gf == f && gh == hole) {
                continue;
            }
            let ids_of = match hole {
                None => flat.ids.clone(),
                Some(h) => flat.hole_ids.get(h).cloned().unwrap_or_default(),
            };
            let mut out: Vec<Segment> = Vec::with_capacity(segs.len() + 2);
            let mut out_ids: Vec<SegId> = Vec::with_capacity(segs.len() + 2);
            let mut first_piece: Vec<usize> = Vec::with_capacity(segs.len());
            for (k, seg) in segs.iter().enumerate() {
                first_piece.push(out.len());
                let id = ids_of.get(k).copied().unwrap_or([0; 4]);
                let Some(list) = groups.get(&(f, hole, k)) else {
                    out.push(seg.clone());
                    out_ids.push(id);
                    continue;
                };
                // The pieces of the edge, named by the bends at their two
                // ends by their flanges' ids, not by their place along it
                // nor in the list (rule 4: a flange added or deleted, on
                // the edge or anywhere in the list, renames only the piece
                // it splits or joins, never another): `part` is
                // `from · 2³² + to`, `from` 0 at the edge's start or the id
                // of the flange whose bend it follows, `to` 0 at the edge's
                // end or the id of the one it runs to; bend `b`'s own
                // tangent line is `id · 2³² + id` (no gap starts and ends
                // at one bend; it is named as the bend's tangent line in
                // any case).
                // A hole's segment named by its place (`[13, flat, hole,
                // segment]`, a cut-out a flange sits on) keeps its place
                // in the third number, so a piece never takes the name of
                // another segment of the hole.
                let part = |from: u64, to: u64| {
                    let between = (from << 32) | (to & 0xffff_ffff);
                    if id[0] == 13 {
                        [16, id[1], (id[2] << 32) | (id[3] & 0xffff_ffff), between]
                    } else {
                        [id[0], id[1], id[2], between]
                    }
                };
                let mark = |b: usize| marks.get(b).copied().unwrap_or(0);
                let d = unit(sub(seg.end, seg.start))
                    .ok_or_else(|| refuse(list.first().copied(), "the edge has no length"))?;
                let along = |p: P2| dot(sub(p, seg.start), d);
                let mut spans: Vec<(f64, f64, usize)> = list
                    .iter()
                    .filter_map(|&b| bends.get(b).map(|bd| (along(bd.start), along(bd.end), b)))
                    .collect();
                spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
                let mut cursor = seg.start;
                let mut next_part = 0u64;
                let mut last: Option<(f64, usize)> = None;
                // How far the relief at the last bend's end reaches along
                // the edge past it (0 without one).
                let mut reached_by_relief = 0.0;
                let floor = SLIVER_TOLERANCES * tol;
                for &(from, to, b) in &spans {
                    if let Some((reached, other)) = last
                        && !(from >= reached + floor)
                    {
                        return Err(refuse(
                            Some(b),
                            format!(
                                "the edge it sits on has a flange already (flange {other}) where this one would be: flanges on one edge need insets that keep them at least {SLIVER_TOLERANCES} tolerances ({floor}) apart"
                            ),
                        ));
                    }
                    let bd = bends
                        .get(b)
                        .ok_or_else(|| refuse(Some(b), "a bend is missing"))?;
                    let number = mark(b)
                        .checked_sub(1)
                        .ok_or_else(|| refuse(Some(b), "it has no id"))?;
                    let into = scale(bd.across, -1.0);
                    if cursor != bd.start {
                        bend_ends.push(BendEnd {
                            bend: b,
                            end: 0,
                            point: bd.start,
                            relief: bend_relief,
                        });
                        let mut stop = bd.start;
                        if let Some(relief) = bend_relief {
                            // The piece before it ends the relief's size
                            // short of the bend and must keep more than a
                            // sliver, also beside the relief at the bend
                            // before.
                            let left = along(bd.start) - relief.size - along(cursor);
                            if !(left >= floor) {
                                return Err(refuse(
                                    Some(b),
                                    format!(
                                        "its bend relief of size {} at its start does not fit the edge: it leaves {left} of the edge before it (at least {floor}, {SLIVER_TOLERANCES} tolerances, must stay){}",
                                        relief.size,
                                        if reached_by_relief > 0.0 {
                                            " beside the relief of the bend before it"
                                        } else {
                                            ""
                                        }
                                    ),
                                ));
                            }
                            stop = sub(bd.start, scale(d, relief.size));
                        }
                        out_ids.push(part(next_part, mark(b)));
                        out.push(piece(seg, cursor, stop));
                        if let Some(relief) = bend_relief {
                            let mut pieces = notch(bd.start, scale(d, -1.0), into, relief)?;
                            let deep = add(bd.start, scale(into, relief.size));
                            pieces.push(line(deep, bd.start));
                            out_ids.extend((0..pieces.len()).map(|j| [11, number, 0, j as u64]));
                            out.extend(pieces);
                        }
                    }
                    flat.seats.push(Seat {
                        bend: b,
                        hole,
                        seg: out.len(),
                    });
                    // A bend spanning the whole edge keeps the edge's own id.
                    let whole = bd.start == seg.start && bd.end == seg.end;
                    out_ids.push(if whole { id } else { part(mark(b), mark(b)) });
                    out.push(piece(seg, bd.start, bd.end));
                    cursor = bd.end;
                    reached_by_relief = 0.0;
                    if bd.end != seg.end {
                        bend_ends.push(BendEnd {
                            bend: b,
                            end: 1,
                            point: bd.end,
                            relief: bend_relief,
                        });
                        if let Some(relief) = bend_relief {
                            let deep = add(bd.end, scale(into, relief.size));
                            let mut pieces = vec![line(bd.end, deep)];
                            pieces.extend(notch(bd.end, into, d, relief)?);
                            out_ids.extend((0..pieces.len()).map(|j| [11, number, 1, j as u64]));
                            out.extend(pieces);
                            cursor = add(bd.end, scale(d, relief.size));
                            reached_by_relief = relief.size;
                        }
                    }
                    next_part = mark(b);
                    last = Some((to, b));
                }
                if cursor != seg.end {
                    if reached_by_relief > 0.0 && !(along(seg.end) - along(cursor) >= floor) {
                        return Err(refuse(
                            last.map(|(_, b)| b),
                            format!(
                                "its bend relief at its end does not fit the edge: it leaves {} of the edge after it (at least {floor}, {SLIVER_TOLERANCES} tolerances, must stay)",
                                along(seg.end) - along(cursor)
                            ),
                        ));
                    }
                    out_ids.push(part(next_part, 0));
                    out.push(piece(seg, cursor, seg.end));
                }
            }
            match hole {
                None => {
                    flat.tip = flat.tip.and_then(|t| first_piece.get(t).copied());
                    flat.was = (0..out.len()).map(Some).collect();
                    flat.pieces = first_piece.clone();
                    flat.pieces.push(out.len());
                    flat.outer = out;
                    flat.ids = out_ids;
                }
                Some(h) => {
                    if let Some(l) = flat.holes.get_mut(h) {
                        *l = out;
                    }
                    if let Some(l) = flat.hole_ids.get_mut(h) {
                        *l = out_ids;
                    }
                }
            }
        }
    }
    Ok(bend_ends)
}

/// `seg` moved by `v`.
fn shifted(seg: &Segment, v: P2) -> Segment {
    let shape = match &seg.shape {
        Shape::Line => Shape::Line,
        Shape::Arc { center, radius } => Shape::Arc {
            center: add(*center, v),
            radius: *radius,
        },
        Shape::Circle { center, radius } => Shape::Circle {
            center: add(*center, v),
            radius: *radius,
        },
        Shape::Spline(spline) => {
            let mut spline = spline.clone();
            for p in &mut spline.control {
                *p = add(*p, v);
            }
            Shape::Spline(spline)
        }
    };
    Segment {
        start: add(seg.start, v),
        end: add(seg.end, v),
        shape,
        ..seg.clone()
    }
}

/// The least and the most of `f(p) = dot(p − o, n)` over the segment
/// (`n` unit): exact for lines, arcs and circles; for a spline the
/// control points' (its convex hull bounds it, so the range holds the
/// curve's).
fn reach_along(seg: &Segment, o: P2, n: P2) -> (f64, f64) {
    let at = |p: P2| dot(sub(p, o), n);
    let (a, b) = (at(seg.start), at(seg.end));
    let ends = (a.min(b), a.max(b));
    match &seg.shape {
        Shape::Line => ends,
        Shape::Circle { center, radius } => (at(*center) - radius, at(*center) + radius),
        Shape::Arc { center, radius } => {
            let Some((from, sweep)) = seg.angles() else {
                return ends;
            };
            let inside = |phi: f64| (phi - from).rem_euclid(TAU) <= sweep;
            let (mut lo, mut hi) = ends;
            if inside(n[1].patan2(n[0])) {
                hi = hi.max(at(*center) + radius);
            }
            if inside((-n[1]).patan2(-n[0])) {
                lo = lo.min(at(*center) - radius);
            }
            (lo, hi)
        }
        Shape::Spline(spline) => spline
            .control
            .iter()
            .map(|&p| at(p))
            .fold(ends, |(lo, hi), x| (lo.min(x), hi.max(x))),
    }
}

/// How far the one line of `profile`'s outer loop on the edge `s → e`
/// (running against it) stands back from the edge's start and its end;
/// `None` when there is not exactly one such line.
fn outline_gaps(profile: &Profile, s: P2, e: P2, tol: f64) -> Option<[f64; 2]> {
    let len = distance(s, e);
    let d = unit(sub(e, s))?;
    let across = [d[1], -d[0]];
    let off = |p: P2| dot(sub(p, s), across);
    let along = |p: P2| dot(sub(p, s), d);
    let mut on_line = profile.outer.segments.iter().filter(|g| {
        matches!(g.shape, Shape::Line)
            && off(g.start).abs() <= tol
            && off(g.end).abs() <= tol
            && along(g.end) < along(g.start)
    });
    let g = on_line.next()?;
    if on_line.next().is_some() {
        return None;
    }
    Some([along(g.end), len - along(g.start)])
}

/// The flat of flange `i` from its own outline `profile`, drawn against
/// its edge `s → e` as if the bend had no width (see [`FlangeSpec`]): the
/// bend's span (the outline's line on the edge), the outer loop moved
/// `shift` on (the bend allowance across) and turned to start at that
/// line, its holes moved likewise, and how far beyond its tangent line it
/// reaches. `lift` is how far the edge lies from where it would with
/// every bend between the base and it of no width (the sum of those
/// bends' allowances, each across its bend): the outline is drawn there,
/// in the edge's own place, so that changing the thickness, a radius or
/// a K-factor moves it with its edge instead of refusing it.
#[allow(clippy::type_complexity)]
fn own_outline(
    i: usize,
    profile: &Profile,
    (s, e): (P2, P2),
    shift: P2,
    lift: P2,
    tol: f64,
) -> Result<(P2, P2, Vec<Segment>, Vec<Vec<Segment>>, f64), KernelError> {
    let len = distance(s, e);
    let d = unit(sub(e, s)).ok_or_else(|| refuse(Some(i), "the edge has no length"))?;
    let across = [d[1], -d[0]];
    // The edge where the outline is drawn.
    let drawn = sub(s, lift);
    let off = |p: P2| dot(sub(p, drawn), across);
    let along = |p: P2| dot(sub(p, drawn), d);
    let segs = &profile.outer.segments;
    let on_line: Vec<usize> = segs
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            matches!(g.shape, Shape::Line)
                && off(g.start).abs() <= tol
                && off(g.end).abs() <= tol
                && along(g.end) < along(g.start)
        })
        .map(|(k, _)| k)
        .collect();
    let k = match on_line.as_slice() {
        [k] => *k,
        [] => {
            return Err(refuse(
                Some(i),
                format!(
                    "the flange's outline has no line on its edge's line (through ({}, {}) along ({}, {})) with the outline beyond it: draw the outline from the edge, as if the bend had no width",
                    s[0], s[1], d[0], d[1]
                ),
            ));
        }
        _ => {
            return Err(refuse(
                Some(i),
                "the flange's outline has several lines on its edge's line; the bend starts from one",
            ));
        }
    };
    let g = segs
        .get(k)
        .ok_or_else(|| refuse(Some(i), "the outline lost its line"))?;
    let (u0, u1) = (along(g.end), along(g.start));
    if !(u0 > -tol && u1 < len + tol) {
        return Err(refuse(
            Some(i),
            format!(
                "the flange's outline runs along its edge from {u0} to {u1}, past the edge's ends (0 to {len}); the bend has to lie on the edge"
            ),
        ));
    }
    for (j, seg) in segs.iter().enumerate() {
        if j != k && reach_along(seg, drawn, across).0 < -tol {
            return Err(refuse(
                Some(i),
                "the flange's outline reaches back across its edge's line; the flat lies beyond its bend",
            ));
        }
    }
    sliver(i, "start", u0, tol)?;
    sliver(i, "end", len - u1, tol)?;
    let start = if u0 <= tol { s } else { add(s, scale(d, u0)) };
    let end = if len - u1 <= tol {
        e
    } else {
        add(s, scale(d, u1))
    };
    let (a0, a1) = (add(start, shift), add(end, shift));
    let moved = add(shift, lift);
    let m = segs.len();
    let mut outer: Vec<Segment> = (0..m)
        .filter_map(|j| segs.get((k + j) % m))
        .map(|seg| shifted(seg, moved))
        .collect();
    // The tangent line exactly where the strip ends, its neighbours meeting
    // it there (moved at most the tolerance).
    if let Some(first) = outer.first_mut() {
        first.start = a1;
        first.end = a0;
    }
    if let Some(second) = outer.get_mut(1) {
        second.start = a0;
    }
    if let Some(last) = outer.last_mut() {
        last.end = a1;
    }
    let holes: Vec<Vec<Segment>> = profile
        .holes
        .iter()
        .map(|h| h.segments.iter().map(|seg| shifted(seg, moved)).collect())
        .collect();
    let flat_length = segs
        .iter()
        .map(|seg| reach_along(seg, drawn, across).1)
        .fold(0.0, f64::max);
    if !(flat_length > tol) {
        return Err(refuse(
            Some(i),
            "the flange's outline has no area beyond its edge",
        ));
    }
    Ok((start, end, outer, holes, flat_length))
}

/// The numbers that name an edge of the folded sheet (and the rim along
/// it), from what the edge is: the same when flanges are added elsewhere,
/// a corner is closed or opened, or a hole is cut elsewhere (rule 4,
/// docs/design/sheetmetal.md › *Names*). `[7, bend, 0]` and `[7, bend, 1]`
/// a bend's tangent lines on its parent's side and its flange's, `[8,
/// bend, end]` the arcs at its ends, `[6, flat, segment]` a piece a closed
/// corner's extension adds, `[9, …, piece]` a piece of a tangent line a
/// hole across a bend crosses, `[10, cut, segment, piece]` a piece of such
/// a hole; any other segment of a flat's outline or hole is its
/// [`SegId`] (`[12, flat, segment]` or `[13, flat, hole, segment]` should
/// a flat not know it); `None` for a flange with no id. A bend is numbered by its flange's id less 1, a
/// flat by 0 (the base) or its flange's id ([`Part::flange_number`],
/// [`Part::flat_number`]), never by a place in the list.
pub fn edge_numbers(part: &Part, key: EdgeKey) -> Option<Vec<u64>> {
    let outline = |flat: usize, seg: usize| -> Option<Vec<u64>> {
        let Some(f) = part.flats.get(flat) else {
            return Some(vec![12, part.flat_number(flat)?, seg as u64]);
        };
        if flat > 0 && seg == 0 {
            return Some(vec![7, part.flange_number(flat - 1)?, 1]);
        }
        // A key counts the outline with every corner open (`was`), wherever
        // the extensions put the segment now.
        if let Some(s) = f
            .seats
            .iter()
            .find(|s| s.hole.is_none() && f.was.get(s.seg).copied().flatten() == Some(seg))
        {
            return Some(vec![7, part.flange_number(s.bend)?, 0]);
        }
        match f.ids.get(seg) {
            Some(id) => Some(id.to_vec()),
            None => Some(vec![12, part.flat_number(flat)?, seg as u64]),
        }
    };
    match key {
        EdgeKey::Outline { flat, seg } => outline(flat, seg),
        EdgeKey::Piece { flat, seg } => Some(vec![6, part.flat_number(flat)?, seg as u64]),
        EdgeKey::Hole { flat, hole, seg } => {
            let f = part.flats.get(flat);
            if let Some(s) = f.and_then(|f| {
                f.seats
                    .iter()
                    .find(|s| s.hole == Some(hole) && s.seg == seg)
            }) {
                return Some(vec![7, part.flange_number(s.bend)?, 0]);
            }
            match f
                .and_then(|f| f.hole_ids.get(hole))
                .and_then(|h| h.get(seg))
            {
                Some(id) => Some(id.to_vec()),
                None => Some(vec![13, part.flat_number(flat)?, hole as u64, seg as u64]),
            }
        }
        EdgeKey::Arc { bend, end } => Some(vec![8, part.flange_number(bend)?, end as u64]),
        EdgeKey::Trim { bend, end } => Some(vec![14, part.flange_number(bend)?, end as u64]),
        EdgeKey::Notch { bend, end, piece } => Some(vec![
            11,
            part.flange_number(bend)?,
            end as u64,
            piece as u64,
        ]),
        EdgeKey::Split { flat, seg, piece } => {
            let mut out = vec![9];
            out.extend(outline(flat, seg)?);
            out.push(piece as u64);
            Some(out)
        }
        EdgeKey::SplitHole {
            flat,
            hole,
            seg,
            piece,
        } => {
            // Named as a split tangent line of the outer loop is: by what
            // the whole segment is (the bend's tangent line) and the piece.
            let mut out = vec![9];
            out.extend(edge_numbers(part, EdgeKey::Hole { flat, hole, seg })?);
            out.push(piece as u64);
            Some(out)
        }
        EdgeKey::Cut { cut, seg, piece } => Some(vec![10, cut as u64, seg as u64, piece as u64]),
    }
}

/// The tip of a flange's flat: from `b0` to `b1`, the flange on its left.
fn tip(bend: &Bend) -> (P2, P2) {
    let h = bend.allowance + bend.flat_length;
    (
        add(bend.start, scale(bend.across, h)),
        add(bend.end, scale(bend.across, h)),
    )
}

/// The notch of a relief at the corner `p` of the base, from
/// `a = p + r ea` to `b = p + r eb` through the base (see the module
/// documentation). `ea` and `eb` are unit, along the two edges away from
/// `p`; the base lies between them, counter-clockwise from `eb` to `ea`.
fn notch(p: P2, ea: P2, eb: P2, relief: Relief) -> Result<Vec<Segment>, KernelError> {
    let r = relief.size;
    let a = add(p, scale(ea, r));
    let b = add(p, scale(eb, r));
    Ok(match relief.kind {
        ReliefKind::Rect => {
            let q = add(a, scale(eb, r));
            vec![line(a, q), line(q, b)]
        }
        ReliefKind::Round => vec![Segment {
            source: 0,
            piece: 0,
            reversed: true,
            start: a,
            end: b,
            shape: Shape::Arc {
                center: p,
                radius: r,
            },
        }],
        ReliefKind::Tear => {
            // The corner's angle α between the edges; the circle tangent
            // to both at distance r from p has its centre on the bisector,
            // r / cos(α/2) from p, and the radius r tan(α/2).
            let alpha = dot(ea, eb).clamp(-1.0, 1.0).pacos();
            let half = 0.5 * alpha;
            let bisector = unit(add(ea, eb)).ok_or_else(|| {
                refuse(
                    None,
                    "a relief at a corner whose edges run back on each other",
                )
            })?;
            let (c, rho) = (half.pcos(), half.ptan());
            if !(c > 1e-9 && rho.is_finite() && rho > 0.0) {
                return Err(refuse(None, "a tear relief at a corner of 180 degrees"));
            }
            vec![Segment {
                source: 0,
                piece: 0,
                reversed: true,
                start: a,
                end: b,
                shape: Shape::Arc {
                    center: add(p, scale(bisector, r / c)),
                    radius: r * rho,
                },
            }]
        }
    })
}

/// The flats folded by the bends: the base stays.
fn fold(flats: &[Flat], bends: &[Bend], tolerance: &Tolerance) -> Result<Folded, KernelError> {
    let mut joints = Vec::with_capacity(bends.len());
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for flat in flats {
        for s in &flat.outer {
            for p in [s.start, s.end] {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
    }
    let size = (hi[0] - lo[0]).phypot(hi[1] - lo[1]);
    for (i, b) in bends.iter().enumerate() {
        let d = unit(sub(b.end, b.start)).ok_or_else(|| refuse(Some(i), "a bend has no length"))?;
        let direction = lift(d);
        let t = b.thickness;
        // The strip starts on the neutral fibre: K T from the inner skin.
        let z = if b.angle > 0.0 { t - b.k * t } else { b.k * t };
        joints.push(Joint {
            left: b.parent,
            right: b.child,
            point: Vec3::new(b.start[0], b.start[1], z),
            direction,
            side: Vec3::Z.cross(direction),
            reach: distance(b.start, b.end),
            id: i as u64,
            radius: b.neutral(),
            across: lift(b.across),
        });
    }
    let link = Linkage {
        bodies: flats.len(),
        joints,
        size: if size.is_finite() && size > 0.0 {
            size
        } else {
            1.0
        },
    };
    let angles: Vec<Angle> = bends.iter().map(|b| Angle::Given(b.angle)).collect();
    fold_linkage(&link, &angles, 0, tolerance.linear, tolerance.angular)
        .map_err(|e| refuse(None, format!("the bends do not fold: {e}")))
}

/// The world transform of a flat: its fold, then the plane.
fn world(plane: &Frame, flat: &Flat) -> Transform {
    flat.transform.then(&Transform::from_frame(plane))
}

/// A loop's vertices and its edges (with whether the loop runs against
/// each).
type Ring = (Vec<VertexId>, Vec<(EdgeId, bool)>);

struct Builder {
    body: Body,
}

impl Builder {
    fn vertex(&mut self, p: Vec3) -> VertexId {
        self.body.add_vertex(p)
    }

    fn point(&self, v: VertexId) -> Option<Vec3> {
        usize::try_from(v.0)
            .ok()
            .and_then(|i| self.body.vertices.get(i))
            .map(|v| v.point)
    }

    fn line(&mut self, a: VertexId, b: VertexId) -> Result<EdgeId, KernelError> {
        let (Some(pa), Some(pb)) = (self.point(a), self.point(b)) else {
            return Err(refuse(None, "a vertex is missing"));
        };
        let l = Line::through(pa, pb)?;
        Ok(self
            .body
            .add_edge(Curve3::Line(l), (0.0, pa.distance(pb)), a, b))
    }

    /// The edge of a pattern segment carried by `w`, between the vertices
    /// where the loop enters and leaves it; and whether the loop runs
    /// against the edge.
    fn segment(
        &mut self,
        seg: &Segment,
        w: &Transform,
        ends: (VertexId, VertexId),
    ) -> Result<(EdgeId, bool), KernelError> {
        let frame_at = |c: P2| -> Result<Frame, KernelError> {
            Frame::new(w.point(lift(c)), w.vector(Vec3::Z), w.vector(Vec3::X))
                .ok_or_else(|| refuse(None, "a flat's plane is not finite"))
        };
        match &seg.shape {
            Shape::Line => Ok((self.line(ends.0, ends.1)?, false)),
            &Shape::Arc { center, radius } => {
                let (begin, sweep) = seg
                    .angles()
                    .ok_or_else(|| refuse(None, "an arc without angles"))?;
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(refuse(None, "an arc whose ends coincide"));
                }
                let circle = Curve3::Circle(Circle::new(frame_at(center)?, radius)?);
                let own = if seg.reversed { (ends.1, ends.0) } else { ends };
                Ok((
                    self.body
                        .add_edge(circle, (begin, begin + sweep), own.0, own.1),
                    seg.reversed,
                ))
            }
            &Shape::Circle { center, radius } => {
                let circle = Curve3::Circle(Circle::new(frame_at(center)?, radius)?);
                Ok((
                    self.body.add_edge(circle, (0.0, TAU), ends.0, ends.1),
                    seg.reversed,
                ))
            }
            Shape::Spline(spline) => {
                let control = spline.control.iter().map(|p| w.point(lift(*p))).collect();
                let curve = NurbsCurve::polynomial(spline.degree, spline.knots.clone(), control)?;
                let range = curve.domain();
                let own = if seg.reversed { (ends.1, ends.0) } else { ends };
                Ok((
                    self.body
                        .add_edge(Curve3::Nurbs(curve), range, own.0, own.1),
                    seg.reversed,
                ))
            }
        }
    }

    /// The edges of a closed loop of pattern segments carried by `w`: a
    /// vertex where each segment starts and an edge per segment, with
    /// whether the loop runs against it.
    fn ring(&mut self, segs: &[Segment], w: &Transform) -> Result<Ring, KernelError> {
        let vertices: Vec<VertexId> = segs
            .iter()
            .map(|s| self.vertex(w.point(lift(s.start))))
            .collect();
        let mut edges = Vec::with_capacity(segs.len());
        for (k, s) in segs.iter().enumerate() {
            let (Some(&a), Some(&b)) = (vertices.get(k), vertices.get((k + 1) % segs.len())) else {
                return Err(refuse(None, "a loop lost a vertex"));
            };
            edges.push(self.segment(s, w, (a, b))?);
        }
        Ok((vertices, edges))
    }
}

fn coedges(edges: &[(EdgeId, bool)]) -> Vec<Coedge> {
    edges
        .iter()
        .map(|&(edge, reversed)| Coedge { edge, reversed })
        .collect()
}

/// The edges of the folded sheet that end an extension at a closed
/// corner, each with the plane (in the world) its end face lies on.
type Ends = Vec<(EdgeId, Surface)>;

/// The folded bottom skin: a planar face per flat, a cylindrical one per
/// bend, sharing the tangent lines' edges; and the extensions' ends.
fn folded_sheet(
    spec: &Spec,
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    tolerance: &Tolerance,
    checked: bool,
) -> Result<(Body, Vec<SheetFace>, Ends, Vec<EdgeKey>), KernelError> {
    if !cuts.is_empty() {
        return cuts::folded_sheet(&spec.plane, flats, bends, cuts, tolerance, checked);
    }
    let mut b = Builder {
        body: Body::new(BodyKind::Sheet),
    };
    let plane = &spec.plane;
    let worlds: Vec<Transform> = flats.iter().map(|f| world(plane, f)).collect();
    let w_of = |i: usize| -> Result<Transform, KernelError> {
        worlds
            .get(i)
            .copied()
            .ok_or_else(|| refuse(None, "a flat is missing"))
    };
    // Per flat: its outer loop's vertices and edges.
    let mut made_rings: Vec<Option<Ring>> = vec![None; flats.len()];
    let mut hole_rings: Vec<Vec<(EdgeId, bool)>> = Vec::new();
    let mut hole_vertices: Vec<Vec<VertexId>> = Vec::new();
    let base = flats.first().ok_or_else(|| refuse(None, "no base"))?;
    let w0 = w_of(0)?;
    if let Some(slot) = made_rings.first_mut() {
        *slot = Some(b.ring(&base.outer, &w0)?);
    }
    for hole in &base.holes {
        let (v, e) = b.ring(hole, &w0)?;
        hole_rings.push(e);
        hole_vertices.push(v);
    }
    // The bends' arcs, and the flanges' flats in order. Flange i's flat
    // shares its tangent edge with bend i's face; its tip may be the
    // parent edge of a later bend.
    let mut bend_loops: Vec<Vec<(EdgeId, bool)>> = vec![Vec::new(); bends.len()];
    // Every bend's end edges, with what each is and the plane a trimmed
    // one's rim lies on.
    let mut bend_edges: Vec<(EdgeId, EdgeKey, Option<Surface>)> = Vec::new();
    // The holes of the flanges' flats: made when a bend sits on one of
    // them (its parent is made by then), the rest after everything a part
    // without such a bend has, so its edges keep their places.
    let mut flange_rings: Vec<Option<Vec<Ring>>> = vec![None; flats.len()];
    for i in parents_first(bends)? {
        let bend = bends
            .get(i)
            .ok_or_else(|| refuse(Some(i), "a bend is missing"))?;
        let rings = &made_rings;
        // The parent edge: `s → e` in the parent's loop (its outer one or
        // a hole's).
        let seat = seat_of(flats, i, bend)?;
        if seat.hole.is_some()
            && bend.parent > 0
            && flange_rings.get(bend.parent).is_some_and(Option::is_none)
        {
            let w = w_of(bend.parent)?;
            let mut made = Vec::new();
            for hole in flats
                .get(bend.parent)
                .map_or(&[][..], |f| f.holes.as_slice())
            {
                made.push(b.ring(hole, &w)?);
            }
            if let Some(slot) = flange_rings.get_mut(bend.parent) {
                *slot = Some(made);
            }
        }
        let (vs, es): (&[VertexId], &[(EdgeId, bool)]) = match seat.hole {
            None => rings
                .get(bend.parent)
                .and_then(Option::as_ref)
                .map(|(v, e)| (v.as_slice(), e.as_slice())),
            Some(h) if bend.parent == 0 => hole_rings
                .get(h)
                .zip(hole_vertices.get(h))
                .map(|(e, v)| (v.as_slice(), e.as_slice())),
            Some(h) => flange_rings
                .get(bend.parent)
                .and_then(Option::as_ref)
                .and_then(|list| list.get(h))
                .map(|(v, e)| (v.as_slice(), e.as_slice())),
        }
        .ok_or_else(|| refuse(Some(i), "a flange's parent edge is missing"))?;
        let k = seat.seg;
        let (ps, pe, parent_edge) = (
            *vs.get(k).ok_or_else(|| refuse(Some(i), "no vertex"))?,
            *vs.get((k + 1) % vs.len().max(1))
                .ok_or_else(|| refuse(Some(i), "no vertex"))?,
            *es.get(k).ok_or_else(|| refuse(Some(i), "no edge"))?,
        );
        let flat = flats
            .get(bend.child)
            .ok_or_else(|| refuse(Some(i), "a flange's flat is missing"))?;
        let wc = w_of(bend.child)?;
        let wp = w_of(bend.parent)?;
        let ring = b.ring(&flat.outer, &wc)?;
        let (Some(&a1), Some(&a0)) = (ring.0.first(), ring.0.get(1)) else {
            return Err(refuse(Some(i), "a flange's flat has no corners"));
        };
        let tangent = *ring
            .1
            .first()
            .ok_or_else(|| refuse(Some(i), "a flange's flat has no edges"))?;
        // The arcs at both ends, about the axis as the parent has it.
        let t = bend.thickness;
        let zc = if bend.angle > 0.0 {
            t + bend.radius
        } else {
            -bend.radius
        };
        let d = lift(unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?);
        let centre = Vec3::new(bend.start[0], bend.start[1], zc);
        let axis_world = wp.vector(d);
        let z = axis_world * (-bend.angle.signum());
        let c0 = wp.point(centre);
        let s0 = wp.point(lift(bend.start));
        let frame = Frame::new(c0, z, s0 - c0)
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let rb = zc.abs();
        let theta = bend.theta();
        let to_world = Transform::from_frame(plane);
        // The bend's end at `end` from the parent's vertex to the
        // flange's, in the order of φ: its arc, or with a trim the arc up
        // to φ₁ and the plane's ellipse beyond (`trim.rs`).
        let side = |b: &mut Builder,
                    from: VertexId,
                    to: VertexId,
                    end: usize,
                    shift: f64|
         -> Result<Vec<(EdgeId, EdgeKey, Option<Surface>)>, KernelError> {
            let f = frame
                .at(frame.origin() + axis_world * shift)
                .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
            let circle = Curve3::Circle(Circle::new(f, rb)?);
            let Some(t) = bend.trim.get(end).copied().flatten() else {
                return Ok(vec![(
                    b.body.add_edge(circle, (0.0, theta), from, to),
                    EdgeKey::Arc { bend: i, end },
                    None,
                )]);
            };
            let (s1, c1) = t.phi.psin_cos();
            let knee = b.vertex(f.origin() + (f.x() * c1 + f.y() * s1) * rb);
            let arc = b.body.add_edge(circle, (0.0, t.phi), from, knee);
            // The plane's ellipse on the bottom skin: centred where the
            // axis meets the plane, semi-axes rb / |n·D| along the axis's
            // projection and rb square to it; its frame turned so that its
            // eccentric angle runs with φ.
            let n = to_world
                .vector(t.normal)
                .normalized()
                .ok_or_else(|| refuse(Some(i), "a trim's plane has no normal"))?;
            let dir = axis_world
                .normalized()
                .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
            let cos = n.dot(dir);
            let centre = wp.point(Vec3::new(bend.start[0], bend.start[1], zc) + d * t.a);
            let x = (dir - n * cos)
                .normalized()
                .ok_or_else(|| refuse(Some(i), "a trim's plane runs along the bend"))?;
            let x_flat = (x - dir * x.dot(dir))
                .normalized()
                .ok_or_else(|| refuse(Some(i), "a trim's plane runs along the bend"))?;
            let (al, be) = (x_flat.dot(f.x()), x_flat.dot(f.y()));
            let want = f.x() * (-be) + f.y() * al;
            let z = if n.cross(x).dot(want) > 0.0 { n } else { -n };
            let ef = Frame::new(centre, z, x)
                .ok_or_else(|| refuse(Some(i), "a trim's ellipse has no frame"))?;
            let ellipse = Curve3::Ellipse(craft3d_geom::Ellipse::new(ef, rb / cos.abs(), rb)?);
            let phase = be.patan2(al);
            let cut = b
                .body
                .add_edge(ellipse, (t.phi - phase, theta - phase), knee, to);
            let onto = Frame::from_z(to_world.point(t.point), n)
                .ok_or_else(|| refuse(Some(i), "a trim's plane is not finite"))?;
            Ok(vec![
                (arc, EdgeKey::Arc { bend: i, end }, None),
                (
                    cut,
                    EdgeKey::Trim { bend: i, end },
                    Some(Surface::Plane(Plane::new(onto))),
                ),
            ])
        };
        let start_side = side(&mut b, ps, a0, 0, 0.0)?;
        let end_side = side(&mut b, pe, a1, 1, distance(bend.start, bend.end))?;
        let mut lp: Vec<(EdgeId, bool)> = start_side.iter().map(|e| (e.0, false)).collect();
        lp.push((tangent.0, !tangent.1));
        lp.extend(end_side.iter().rev().map(|e| (e.0, true)));
        lp.push((parent_edge.0, !parent_edge.1));
        if let Some(slot) = bend_loops.get_mut(i) {
            *slot = lp;
        }
        bend_edges.extend(start_side.into_iter().chain(end_side));
        if let Some(slot) = made_rings.get_mut(bend.child) {
            *slot = Some(ring);
        }
    }
    let rings: Vec<Ring> = made_rings
        .into_iter()
        .collect::<Option<_>>()
        .ok_or_else(|| refuse(None, "a flat of the part hangs from no bend"))?;

    // The extensions' ends, with their planes in the world.
    let to_world = Transform::from_frame(plane);
    let mut ends: Ends = Vec::new();
    for (f, flat) in flats.iter().enumerate() {
        for end in &flat.ends {
            let edge = rings
                .get(f)
                .and_then(|(_, es)| es.get(end.segment))
                .ok_or_else(|| refuse(f.checked_sub(1), "an extension's end has no edge"))?;
            let frame = Frame::from_z(to_world.point(end.point), to_world.vector(end.normal))
                .ok_or_else(|| {
                    refuse(f.checked_sub(1), "an extension's end plane is not finite")
                })?;
            ends.push((edge.0, Surface::Plane(Plane::new(frame))));
        }
    }

    for (edge, _, onto) in &bend_edges {
        if let Some(surface) = onto {
            ends.push((*edge, surface.clone()));
        }
    }

    // The holes of the flanges' flats (after everything a part without
    // them has, so its edges keep their places).
    let mut flange_holes: Vec<Vec<Vec<(EdgeId, bool)>>> = vec![Vec::new(); flats.len()];
    for (f, flat) in flats.iter().enumerate().skip(1) {
        if let Some(Some(made)) = flange_rings.get_mut(f) {
            if let Some(list) = flange_holes.get_mut(f) {
                *list = std::mem::take(made).into_iter().map(|r| r.1).collect();
            }
            continue;
        }
        let w = w_of(f)?;
        for hole in &flat.holes {
            let ring = b.ring(hole, &w)?.1;
            if let Some(list) = flange_holes.get_mut(f) {
                list.push(ring);
            }
        }
    }

    // Faces: the flats, then the bends.
    let mut faces: Vec<FaceId> = Vec::with_capacity(flats.len() + bends.len());
    let mut roles = Vec::with_capacity(flats.len() + bends.len());
    for (i, ((_, edges), w)) in rings.iter().zip(&worlds).enumerate() {
        let mut loops = vec![b.body.add_loop(coedges(edges))];
        let holes = if i == 0 {
            hole_rings.as_slice()
        } else {
            flange_holes.get(i).map_or(&[][..], Vec::as_slice)
        };
        for hole in holes {
            loops.push(b.body.add_loop(coedges(hole)));
        }
        let frame = w
            .frame(&Frame::WORLD)
            .ok_or_else(|| refuse(None, "a flat's plane is not finite"))?;
        faces.push(
            b.body
                .add_face(Surface::Plane(Plane::new(frame)), false, loops),
        );
        roles.push(SheetFace::Flat(i));
    }
    for (i, (bend, edges)) in bends.iter().zip(&bend_loops).enumerate() {
        let wp = w_of(bend.parent)?;
        let t = bend.thickness;
        let zc = if bend.angle > 0.0 {
            t + bend.radius
        } else {
            -bend.radius
        };
        let d = lift(unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(Some(i), "no length"))?);
        let axis_world = wp.vector(d);
        let c0 = wp.point(Vec3::new(bend.start[0], bend.start[1], zc));
        let s0 = wp.point(lift(bend.start));
        let frame = Frame::new(c0, axis_world * (-bend.angle.signum()), s0 - c0)
            .ok_or_else(|| refuse(Some(i), "a bend's axis is not finite"))?;
        let surface = Surface::Cylinder(Cylinder::new(frame, zc.abs())?);
        let lp = b.body.add_loop(coedges(edges));
        // The bottom skin faces into the material: towards the axis for a
        // bend up (the bottom is the outside), away from it for a bend down.
        faces.push(b.body.add_face(surface, bend.angle > 0.0, vec![lp]));
        roles.push(SheetFace::Bend(i));
    }
    b.body.add_shell(faces);
    // What each edge is: the outlines' segments by their open places, the
    // holes', the bends' arcs.
    let mut keys: Vec<Option<EdgeKey>> = vec![None; b.body.edges.len()];
    let mut put = |edge: EdgeId, key: EdgeKey| {
        if let Some(slot) = usize::try_from(edge.0).ok().and_then(|e| keys.get_mut(e)) {
            *slot = Some(key);
        }
    };
    for (flat, (_, edges)) in rings.iter().enumerate() {
        for (seg, (edge, _)) in edges.iter().enumerate() {
            put(*edge, outline_key(flats, flat, seg));
        }
    }
    for (hole, edges) in hole_rings.iter().enumerate() {
        for (seg, (edge, _)) in edges.iter().enumerate() {
            put(*edge, EdgeKey::Hole { flat: 0, hole, seg });
        }
    }
    // A cut-out placed inside an extension has a key no part with its
    // corners open has (with the corners open it lies in no flat and that
    // part is refused): it is named while the corner stays closed.
    for (flat, holes) in flange_holes.iter().enumerate() {
        for (hole, edges) in holes.iter().enumerate() {
            for (seg, (edge, _)) in edges.iter().enumerate() {
                put(*edge, EdgeKey::Hole { flat, hole, seg });
            }
        }
    }
    for (edge, key, _) in &bend_edges {
        put(*edge, *key);
    }
    let keys: Vec<EdgeKey> = keys
        .into_iter()
        .collect::<Option<_>>()
        .ok_or_else(|| refuse(None, "an edge of the folded sheet is not accounted for"))?;
    if !checked {
        return Ok((b.body, roles, ends, keys));
    }
    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sheet metal",
            defects,
        });
    }
    Ok((b.body, roles, ends, keys))
}

/// The bend's neutral band, `R + K T` about its axis over its angle and
/// as long as the bend, as a sheet of one face: what [`develop`] lays
/// flat to measure the strip's width.
pub fn neutral_band(bend: &Bend) -> Result<Body, KernelError> {
    let r = bend.neutral();
    let theta = bend.theta();
    let w = distance(bend.start, bend.end);
    let mut body = Body::new(BodyKind::Sheet);
    let frame = Frame::WORLD;
    let at = |u: f64, v: f64| Vec3::new(r * u.pcos(), r * u.psin(), v);
    let v00 = body.add_vertex(at(0.0, 0.0));
    let v10 = body.add_vertex(at(theta, 0.0));
    let v11 = body.add_vertex(at(theta, w));
    let v01 = body.add_vertex(at(0.0, w));
    let bottom = body.add_edge(
        Curve3::Circle(Circle::new(frame, r)?),
        (0.0, theta),
        v00,
        v10,
    );
    let top_frame = frame
        .at(Vec3::new(0.0, 0.0, w))
        .ok_or_else(|| refuse(None, "a band's frame is not finite"))?;
    let top = body.add_edge(
        Curve3::Circle(Circle::new(top_frame, r)?),
        (0.0, theta),
        v01,
        v11,
    );
    let side = |body: &mut Body,
                a: VertexId,
                b: VertexId,
                pa: Vec3,
                pb: Vec3|
     -> Result<EdgeId, KernelError> {
        Ok(body.add_edge(
            Curve3::Line(Line::through(pa, pb)?),
            (0.0, pa.distance(pb)),
            a,
            b,
        ))
    };
    let far = side(&mut body, v10, v11, at(theta, 0.0), at(theta, w))?;
    let near = side(&mut body, v00, v01, at(0.0, 0.0), at(0.0, w))?;
    let lp = body.add_loop(vec![
        Coedge {
            edge: bottom,
            reversed: false,
        },
        Coedge {
            edge: far,
            reversed: false,
        },
        Coedge {
            edge: top,
            reversed: true,
        },
        Coedge {
            edge: near,
            reversed: true,
        },
    ]);
    let face = body.add_face(Surface::Cylinder(Cylinder::new(frame, r)?), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}

/// A bend's allowance as the development of its neutral band measures
/// it: the flat length of the band's arcs (both of them; the larger).
pub fn developed_allowance(bend: &Bend, tolerance: &Tolerance) -> Result<f64, KernelError> {
    let band = neutral_band(bend)?;
    let dev = develop(&band, &[FaceId(0)], tolerance)?;
    let mut widths = Vec::new();
    for curve in dev.outline() {
        let Some(edge) = curve.edge else { continue };
        let is_arc = band
            .edges
            .get(edge.0 as usize)
            .is_some_and(|e| matches!(e.curve, Curve3::Circle(_)));
        if is_arc {
            widths.push(curve.length_flat);
        }
    }
    widths
        .into_iter()
        .reduce(f64::max)
        .ok_or_else(|| refuse(None, "the band's development has no arcs"))
}

/// The inner radius of a bend as the solid has it: the smaller radius of
/// the two skins the plate made of the bend's face.
pub fn measured_radius(part: &Part, bend: usize) -> Option<f64> {
    let face = part
        .sheet_faces
        .iter()
        .position(|f| *f == SheetFace::Bend(bend))
        .and_then(|f| u32::try_from(f).ok())
        .map(FaceId)?;
    let mut radii = Vec::new();
    for (i, made) in part.solid.faces.iter().enumerate() {
        let of = match made {
            crate::plate::Made::Front(f) | crate::plate::Made::Back(f) => *f,
            crate::plate::Made::Rim(_) => continue,
        };
        if of != face {
            continue;
        }
        if let Some(Surface::Cylinder(c)) = part.solid.body.faces.get(i).map(|f| &f.surface) {
            radii.push(c.radius());
        }
    }
    radii.into_iter().reduce(f64::min)
}

/// The faces of the part (indices into `part.solid.body`) made from the
/// flat `flat` (0 the base, `i + 1` flange `i`'s): its two skins and the
/// rims along its outline's edges that end the sheet.
pub fn flat_faces(part: &Part, flat: usize) -> Vec<usize> {
    let faces: Vec<FaceId> = part
        .sheet_faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, SheetFace::Flat(x) | SheetFace::FlatPiece(x, _) if *x == flat))
        .filter_map(|(f, _)| u32::try_from(f).ok().map(FaceId))
        .collect();
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

/// The angle between the planes of a bend's two flats, as the fold put
/// them: θ back.
pub fn measured_angle(part: &Part, bend: usize) -> Option<f64> {
    let b = part.bends.get(bend)?;
    let np = part.flats.get(b.parent)?.transform.vector(Vec3::Z);
    let nc = part.flats.get(b.child)?.transform.vector(Vec3::Z);
    Some(np.dot(nc).clamp(-1.0, 1.0).pacos())
}

/// How a hole of a flat lies to a bend beside that flat: the least
/// distance in the pattern from the hole's boundary to the bend's tangent
/// line on the flat's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoleToBend {
    /// The flat (0 the base, `i + 1` flange `i`'s), the hole (its index
    /// among the flat's holes) and the bend.
    pub flat: usize,
    pub hole: usize,
    pub bend: usize,
    /// The least distance: in closed form for lines, arcs and circles; for
    /// a spline the least distance from the chords of its pieces once they
    /// are flat within an eighth of the tolerance, so within an eighth of
    /// the tolerance of the true one either way (not a distance at a place
    /// on the curve) ...
    pub distance: f64,
    /// ... what the true one cannot be below: equal to `distance` in
    /// closed form, an eighth of the tolerance less for a spline, 0 when
    /// the search ran out of work ...
    pub lower: f64,
    /// ... and what it cannot be above: equal to `distance` in closed
    /// form, an eighth of the tolerance more for a spline, infinite when
    /// the search ran out of work (a piece left unrefined proves nothing).
    pub upper: f64,
    /// True when `distance` is in closed form (no spline in the hole).
    pub exact: bool,
}

/// For every hole of every flat (the base, then the flanges in order),
/// the distance to the nearest bend beside that flat: to the tangent line
/// on the flat's side of every bend that starts from it (its parent's
/// edge) or ends on it (a flange's own bend, its far tangent line). A flat
/// is rigid, so the distance in the pattern is the distance on the part.
/// Holes beside no bend are not listed.
pub fn holes_to_bends(part: &Part, tolerance: &Tolerance) -> Vec<HoleToBend> {
    let mut out = Vec::new();
    for (f, flat) in part.flats.iter().enumerate() {
        holes_of_flat(part, f, flat, tolerance, &mut out);
    }
    out
}

fn holes_of_flat(
    part: &Part,
    f: usize,
    flat: &Flat,
    tolerance: &Tolerance,
    out: &mut Vec<HoleToBend>,
) {
    for (h, hole) in flat.holes.iter().enumerate() {
        // A hole a flange grows into is that bend's edge, not a hole
        // beside it.
        if flat.seats.iter().any(|s| s.hole == Some(h)) {
            continue;
        }
        let mut best: Option<HoleToBend> = None;
        for (i, bend) in part.bends.iter().enumerate() {
            let (a, b) = if bend.parent == f {
                (bend.start, bend.end)
            } else if bend.child == f {
                bend.far_line()
            } else {
                continue;
            };
            let mut found = HoleToBend {
                flat: f,
                hole: h,
                bend: i,
                distance: f64::INFINITY,
                lower: f64::INFINITY,
                upper: f64::INFINITY,
                exact: true,
            };
            for seg in hole {
                let (d, lower, upper, exact) = segment_to_line(seg, a, b, tolerance.linear);
                found.distance = found.distance.min(d);
                found.lower = found.lower.min(lower);
                found.upper = found.upper.min(upper);
                found.exact &= exact;
            }
            if best.is_none_or(|b| found.lower < b.lower) {
                best = Some(found);
            }
        }
        out.extend(best);
    }
}

fn point_to_segment(p: P2, a: P2, b: P2) -> (f64, P2) {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = add(a, scale(d, t));
    (distance(p, q), q)
}

/// The least distance from a segment of a loop to the straight segment
/// `a`–`b` they do not cross: the distance found, what the true one cannot
/// be below and above, and whether it is in closed form (lines, arcs,
/// circles) or searched (splines, to an eighth of `tol`; without bounds,
/// 0 and infinity, when the search ran out of work).
fn segment_to_line(seg: &Segment, a: P2, b: P2, tol: f64) -> (f64, f64, f64, bool) {
    let closed = |d: f64| (d, d, d, true);
    match &seg.shape {
        Shape::Line => closed(
            point_to_segment(seg.start, a, b)
                .0
                .min(point_to_segment(seg.end, a, b).0)
                .min(seg.distance_to(a, tol))
                .min(seg.distance_to(b, tol)),
        ),
        Shape::Circle { center, radius } => {
            closed((point_to_segment(*center, a, b).0 - radius).max(0.0))
        }
        Shape::Arc { center, radius } => {
            let mut d = point_to_segment(seg.start, a, b)
                .0
                .min(point_to_segment(seg.end, a, b).0)
                .min(seg.distance_to(a, tol))
                .min(seg.distance_to(b, tol));
            // The arc's point nearest the line, where the normal from the
            // segment's nearest point to the centre meets it.
            let (dc, q) = point_to_segment(*center, a, b);
            if let Some(dir) = unit(sub(q, *center)) {
                let on = add(*center, scale(dir, *radius));
                if seg.distance_to(on, tol) <= tol {
                    d = d.min((dc - radius).abs());
                }
            }
            closed(d)
        }
        Shape::Spline(_) => match seg.spline_to_line(a, b, tol) {
            Some((d, true)) => (d, (d - tol / 8.0).max(0.0), d + tol / 8.0, false),
            Some((d, false)) => (d, 0.0, f64::INFINITY, false),
            None => (f64::INFINITY, 0.0, f64::INFINITY, false),
        },
    }
}

/// The flange length of a flange's flat: from its bend's tangent line on
/// the flange's side to the farthest point of its outline.
pub fn flange_length(part: &Part, bend: usize) -> Option<f64> {
    let b = part.bends.get(bend)?;
    let flat = part.flats.get(b.child)?;
    let (far, _) = b.far_line();
    flat.outer
        .iter()
        .map(|s| reach_along(s, far, b.across).1)
        .reduce(f64::max)
}

/// The area of the flat pattern: the flats' (holes taken out) and the
/// bends' strips.
pub fn flat_area(part: &Part) -> f64 {
    pattern_area(&part.flats, &part.bends, &part.cuts)
}

fn pattern_area(flats: &[Flat], bends: &[Bend], cuts: &[Cut]) -> f64 {
    let ring = |segs: &[Segment]| {
        crate::profile::Loop2 {
            segments: segs.to_vec(),
        }
        .signed_area()
    };
    let flats: f64 = flats
        .iter()
        .map(|f| ring(&f.outer) + f.holes.iter().map(|h| ring(h)).sum::<f64>())
        .sum();
    let strips: f64 = bends
        .iter()
        .map(|b| {
            b.allowance * distance(b.start, b.end)
                - b.trim.iter().flatten().map(|t| t.area).sum::<f64>()
        })
        .sum();
    let cuts: f64 = cuts.iter().map(|c| ring(&c.segments)).sum();
    flats + strips + cuts
}

/// The outline of the flat pattern, in the pattern's axes: closed chains
/// of segments, the outer one counter-clockwise and the holes clockwise
/// (each segment's `start` is where the one before it ends). The flats'
/// edges that a bend's strip continues are left out; the strips' ends
/// are lines across them.
pub fn outline(part: &Part, tolerance: &Tolerance) -> Vec<Vec<Segment>> {
    tagged_outline(&part.flats, &part.bends, &part.cuts, tolerance.linear)
        .into_iter()
        .map(|chain| chain.into_iter().map(|(s, _)| s).collect())
        .collect()
}

/// The box of the flat pattern in the pattern's axes, `[min x, min y, max
/// x, max y]`, and whether it is exact: lines, arcs and circles are held
/// by their ends and extremes (exact), a spline by its control points (a
/// box that holds it, not exact). `None` for a pattern with no outline.
pub fn flat_box(part: &Part, tolerance: &Tolerance) -> Option<([f64; 4], bool)> {
    let chains = outline(part, tolerance);
    let mut exact = true;
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for s in chains.iter().flatten() {
        exact &= !matches!(s.shape, Shape::Spline(_));
        let o = s.bbox();
        b = [
            b[0].min(o[0]),
            b[1].min(o[1]),
            b[2].max(o[2]),
            b[3].max(o[3]),
        ];
    }
    b.iter().all(|x| x.is_finite()).then_some((b, exact))
}

/// The strips (by bend) and the flats a hole across a bend may lie in:
/// those whose box in the pattern the box of some cut meets (within the
/// tolerance). A box holds its curve, so a strip or flat left out is cut
/// by no hole across a bend: its slab, or its outline, is its material
/// exactly. Used where a piece is stood for by its slab or outline
/// (the press-brake check), to know where that is more than the material.
pub fn cut_pieces(
    part: &Part,
    tolerance: &Tolerance,
) -> (
    std::collections::BTreeSet<usize>,
    std::collections::BTreeSet<usize>,
) {
    let tol = tolerance.linear;
    let meets = |a: [f64; 4], b: [f64; 4]| {
        !(a[0] > b[2] + tol || a[2] < b[0] - tol || a[1] > b[3] + tol || a[3] < b[1] - tol)
    };
    let boxes: Vec<[f64; 4]> = part
        .cuts
        .iter()
        .map(|c| {
            crate::profile::Loop2 {
                segments: c.segments.clone(),
            }
            .bbox()
        })
        .collect();
    let mut strips = std::collections::BTreeSet::new();
    for (i, b) in part.bends.iter().enumerate() {
        let (f0, f1) = b.far_line();
        let xs = [b.start[0], b.end[0], f0[0], f1[0]];
        let ys = [b.start[1], b.end[1], f0[1], f1[1]];
        let rect = [
            xs.iter().copied().fold(f64::INFINITY, f64::min),
            ys.iter().copied().fold(f64::INFINITY, f64::min),
            xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ];
        // A trimmed bend's slab (and ring sector) is more than its
        // material too.
        if b.trim.iter().any(Option::is_some) || boxes.iter().any(|c| meets(*c, rect)) {
            strips.insert(i);
        }
    }
    let mut flats = std::collections::BTreeSet::new();
    for (f, flat) in part.flats.iter().enumerate() {
        let fb = crate::profile::Loop2 {
            segments: flat.outer.clone(),
        }
        .bbox();
        if boxes.iter().any(|c| meets(*c, fb)) {
            flats.insert(f);
        }
    }
    (strips, flats)
}

/// [`outline`] with, per segment, the flat it belongs to (0 the base,
/// `i + 1` flange `i`, whose bend's strip is counted with it; a cut across
/// bends counts with the base).
fn tagged_outline(
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    tol: f64,
) -> Vec<Vec<(Segment, usize)>> {
    // The edges a strip continues: per flat, its loop (`None` the outer
    // one) and segment.
    let mut inner: BTreeMap<(usize, Option<usize>, usize), ()> = BTreeMap::new();
    for (i, b) in bends.iter().enumerate() {
        // The child's tangent edge is its segment 0.
        inner.insert((b.child, None, 0), ());
        if let Ok(seat) = seat_of(flats, i, b) {
            inner.insert((b.parent, seat.hole, seat.seg), ());
        }
    }
    let mut pieces: Vec<(Segment, usize)> = Vec::new();
    let mut loops: Vec<Vec<(Segment, usize)>> = Vec::new();
    for (f, flat) in flats.iter().enumerate() {
        for (k, s) in flat.outer.iter().enumerate() {
            if !inner.contains_key(&(f, None, k)) {
                pieces.push((s.clone(), f));
            }
        }
        for (h, hole) in flat.holes.iter().enumerate() {
            if flat.seats.iter().any(|s| s.hole == Some(h)) {
                // A hole a flange grows into: chained with the strips.
                for (k, s) in hole.iter().enumerate() {
                    if !inner.contains_key(&(f, Some(h), k)) {
                        pieces.push((s.clone(), f));
                    }
                }
            } else {
                loops.push(hole.iter().map(|s| (s.clone(), f)).collect());
            }
        }
    }
    for b in bends {
        for end in 0..2 {
            for seg in trim::strip_end(b, end) {
                pieces.push((seg, b.child));
            }
        }
    }
    // Chained head to tail.
    let mut outer: Vec<Vec<(Segment, usize)>> = Vec::new();
    let mut used = vec![false; pieces.len()];
    for first in 0..pieces.len() {
        if used.get(first).copied().unwrap_or(true) {
            continue;
        }
        let Some(seg) = pieces.get(first).cloned() else {
            continue;
        };
        if let Some(u) = used.get_mut(first) {
            *u = true;
        }
        let start = seg.0.start;
        let mut chain = vec![seg];
        let mut guard = 0usize;
        while guard <= pieces.len() {
            guard += 1;
            let Some(end) = chain.last().map(|(s, _)| s.end) else {
                break;
            };
            if chain.len() > 1 && distance(end, start) <= tol {
                break;
            }
            let next = (0..pieces.len()).find(|&j| {
                !used.get(j).copied().unwrap_or(true)
                    && pieces
                        .get(j)
                        .is_some_and(|(s, _)| distance(s.start, end) <= tol)
            });
            let Some(j) = next else { break };
            if let Some(u) = used.get_mut(j) {
                *u = true;
            }
            if let Some(s) = pieces.get(j) {
                chain.push(s.clone());
            }
        }
        outer.push(chain);
    }
    outer.extend(loops);
    outer.extend(
        cuts.iter()
            .map(|c| c.segments.iter().map(|s| (s.clone(), 0)).collect()),
    );
    outer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use crate::spline2::SEGMENT_WORK_LIMIT;

    /// A 60 × 40 base with a hole bounded by a cubic spline and a line,
    /// and one flange on its bottom edge, at the scale `k`.
    fn spline_hole_part(k: f64, tol: &Tolerance) -> Part {
        let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let c = [[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]];
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: s(a),
                end: s(b),
            },
        };
        let mut curves: Vec<SketchCurve> = (0..4)
            .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
            .collect();
        curves.push(line(9, [40.0, 15.0], [20.0, 15.0]));
        curves.push(SketchCurve {
            id: 10,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: [[20.0, 15.0], [24.0, 2.0], [33.0, 4.0], [40.0, 15.0]]
                    .iter()
                    .map(|p| s(*p))
                    .collect(),
            }),
        });
        let base = detect_profiles(&curves, tol).unwrap().remove(0);
        build(
            &Spec {
                base,
                plane: Frame::WORLD,
                thickness: k,
                flanges: vec![FlangeSpec {
                    on: On::Base(1),
                    length: 10.0 * k,
                    angle: 0.5 * PI,
                    direction: Direction::Up,
                    radius: k,
                    k: 0.4,
                    from: LengthFrom::Tangent,
                    inset: [0.0; 2],
                    outline: None,
                }],
                relief: None,
                bend_relief: None,
                corner: Closing::OPEN,
                corners: Vec::new(),
                cutouts: Vec::new(),
                jogs: Vec::new(),
                ids: Vec::new(),
            },
            tol,
        )
        .unwrap()
    }

    /// A spline hole's distance to the bend is bounded both ways while the
    /// search is proven (an eighth of the tolerance), and not bounded
    /// above at all when the search runs out of work: the spline's upper
    /// bound is infinite then, so the hole's is only what its line proves
    /// (15 from the bend line, in closed form), never the unproven search's
    /// estimate plus an eighth.
    #[test]
    fn a_spline_search_that_runs_out_of_work_bounds_nothing_above() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let part = spline_hole_part(k, &tol);
            let found = holes_to_bends(&part, &tol);
            assert_eq!(found.len(), 1);
            let h = found[0];
            assert!(!h.exact);
            let eighth = tol.linear / 8.0;
            assert!(
                (h.upper - (h.distance + eighth)).abs() <= 1e-12 * k,
                "{h:?}"
            );
            assert!((h.distance - eighth - h.lower).abs() <= 1e-12 * k, "{h:?}");

            let before = SEGMENT_WORK_LIMIT.with(|c| c.replace(2));
            let starved = holes_to_bends(&part, &tol);
            let bend = part.bends[0];
            let spline = part.flats[0].holes[0]
                .iter()
                .find(|s| matches!(s.shape, Shape::Spline(_)))
                .unwrap()
                .clone();
            let (d, lower, upper, exact) =
                segment_to_line(&spline, bend.start, bend.end, tol.linear);
            SEGMENT_WORK_LIMIT.with(|c| c.set(before));
            assert!(!exact && d.is_finite());
            assert_eq!((lower, upper), (0.0, f64::INFINITY));
            assert_eq!(starved.len(), 1);
            let h = starved[0];
            assert!(!h.exact);
            assert_eq!(h.lower, 0.0, "{h:?}");
            assert!(h.distance + eighth < 15.0 * k, "{h:?}");
            assert!((h.upper - 15.0 * k).abs() <= 1e-12 * k, "{h:?}");
        }
    }
}
