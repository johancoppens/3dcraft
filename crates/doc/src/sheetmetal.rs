//! A sheet-metal part: a base flat (the closed profile of a sketch) with
//! flanges bent along its straight edges and along the tips of earlier
//! flanges, each bend with an inner radius and a K-factor, and corner
//! reliefs (docs/design/sheetmetal.md).

use crate::{EntityId, Expr, FeatureId};
use serde::{Deserialize, Serialize};

/// The most flanges one part may have.
pub const MAX_SHEET_FLANGES: usize = 1024;

/// Which way a bend turns its flange, seen from the side the sketch's
/// normal points to: `up` towards it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BendDirection {
    Up,
    Down,
}

impl BendDirection {
    pub fn name(self) -> &'static str {
        match self {
            BendDirection::Up => "up",
            BendDirection::Down => "down",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "up" => Some(BendDirection::Up),
            "down" => Some(BendDirection::Down),
            _ => None,
        }
    }
}

/// Where a flange's length is measured from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlangeFrom {
    /// From the bend's tangent line: the flange's flat is this long.
    #[default]
    Tangent,
    /// From the virtual sharp of the outside surfaces (the outside mould
    /// line).
    Mould,
}

impl FlangeFrom {
    pub fn name(self) -> &'static str {
        match self {
            FlangeFrom::Tangent => "tangent",
            FlangeFrom::Mould => "mould",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "tangent" => Some(FlangeFrom::Tangent),
            "mould" => Some(FlangeFrom::Mould),
            _ => None,
        }
    }
}

/// The straight edge a flange sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlangeOn {
    /// The edge of the base made by this line of the sketch.
    Base(EntityId),
    /// The tip of the flange at this place in the list (an earlier one).
    Tip(u32),
    /// A side of the flange at this place in the list (an earlier one):
    /// its edge at the start or at the end of its bend.
    Side(u32, FlangeSide),
    /// A straight line of the own outline of the flange at this place in
    /// the list (an earlier one with `outline`), by the line's entity id
    /// in that outline's sketch (format version 52, a TEMPORARY number).
    Line(u32, EntityId),
}

/// Which side of a flange's flat another flange sits on: the one at the
/// start of its bend (its edge's start) or at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlangeSide {
    Start,
    End,
}

impl FlangeSide {
    pub fn name(self) -> &'static str {
        match self {
            FlangeSide::Start => "start",
            FlangeSide::End => "end",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "start" => Some(FlangeSide::Start),
            "end" => Some(FlangeSide::End),
            _ => None,
        }
    }
}

/// One flange: a flat bent from an edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flange {
    pub on: FlangeOn,
    pub length: Expr,
    /// The bend angle, more than 0 and at most 180 degrees.
    pub angle: Expr,
    pub direction: BendDirection,
    /// The bend's inner radius; `None`: the part's.
    pub radius: Option<Expr>,
    /// The bend's K-factor; `None`: the part's.
    pub k_factor: Option<Expr>,
    pub from: FlangeFrom,
    /// How far the bend stands back from its edge's start and end (`None`:
    /// 0, the bend spans the edge).
    pub inset_start: Option<Expr>,
    pub inset_end: Option<Expr>,
    /// A sketch whose closed profile is the flange's flat, drawn in the
    /// base sketch's plane as if the bend had no width: one line of it on
    /// the edge's line, the rest beyond (`None`: the rectangle `length`
    /// long). Comes before the part in the history.
    pub outline: Option<FeatureId>,
    /// What the part names this flange's bend by where its edge is split
    /// at it (the pieces between bends), so those names stay with the
    /// flange when others are added, deleted or moved in the list: above
    /// 0, no two flanges of a part alike, never one a flange deleted had
    /// ([`SheetMetal::next_flange_id`]). A file without ids reads each
    /// as its place in the list plus 1 (what those names were).
    #[serde(default)]
    pub id: u32,
}

/// The shape of a corner relief.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReliefShape {
    Rect,
    Round,
    Tear,
}

impl ReliefShape {
    pub fn name(self) -> &'static str {
        match self {
            ReliefShape::Rect => "rect",
            ReliefShape::Round => "round",
            ReliefShape::Tear => "tear",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "rect" => Some(ReliefShape::Rect),
            "round" => Some(ReliefShape::Round),
            "tear" => Some(ReliefShape::Tear),
            _ => None,
        }
    }
}

/// A choice among the names of `T`: one of them as written, or an
/// expression whose value picks one (1 for the first, as a published
/// choice parameter counts: `docs/guide/05-jigs.md`), so a Jig's choice or
/// a table lookup can drive it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pick<T> {
    Named(T),
    Expr(Expr),
}

impl<T> Pick<T> {
    pub fn expr(&self) -> Option<&Expr> {
        match self {
            Pick::Named(_) => None,
            Pick::Expr(e) => Some(e),
        }
    }

    fn expr_mut(&mut self) -> Option<&mut Expr> {
        match self {
            Pick::Named(_) => None,
            Pick::Expr(e) => Some(e),
        }
    }
}

/// The relief cut at every corner of the base where two flanged edges
/// meet: its shape (rect 1, round 2, tear 3) and how far along each edge
/// from the corner it reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CornerRelief {
    pub shape: Pick<ReliefShape>,
    pub size: Expr,
}

/// How a corner where two flanged edges meet is closed
/// (docs/design/sheetmetal.md › *Closed corners*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CornerKind {
    Open,
    Butt,
    Overlap,
    Miter,
}

impl CornerKind {
    /// In the order a choice counts them (1 open .. 4 miter).
    pub const ALL: [CornerKind; 4] = [
        CornerKind::Open,
        CornerKind::Butt,
        CornerKind::Overlap,
        CornerKind::Miter,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CornerKind::Open => "open",
            CornerKind::Butt => "butt",
            CornerKind::Overlap => "overlap",
            CornerKind::Miter => "miter",
        }
    }

    /// The kind a name names; `mitre` (the spelling `feature.plate`'s
    /// joints use) is `miter` too.
    pub fn from_name(name: &str) -> Option<Self> {
        if name == "mitre" {
            return Some(CornerKind::Miter);
        }
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

impl ReliefShape {
    /// In the order a choice counts them (1 rect, 2 round, 3 tear).
    pub const ALL: [ReliefShape; 3] = [ReliefShape::Rect, ReliefShape::Round, ReliefShape::Tear];
}

/// The part's treatment of its corners: the kind and the gap between the
/// two flats (needed when the kind is not open).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CornerClose {
    pub kind: Pick<CornerKind>,
    pub gap: Option<Expr>,
}

/// The treatment of one corner, named by the two flanges that meet there
/// (their places in the list, either order): its kind, its gap (`None`:
/// the part's) and, for butt and overlap, the flange that covers the
/// corner (`None`: the one on the edge arriving at the corner).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetCorner {
    pub flanges: [u32; 2],
    pub kind: Pick<CornerKind>,
    pub gap: Option<Expr>,
    pub over: Option<u32>,
}

/// A sheet-metal part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetMetal {
    /// The sketch whose closed profile the flanges' edges are on is the
    /// base flat (holes allowed); its other closed profiles are cut-outs
    /// (below). It has to come before this feature in the history.
    pub sketch: FeatureId,
    pub thickness: Expr,
    /// The inner radius of the bends that do not give their own.
    pub radius: Expr,
    /// The K-factor of the bends that do not give their own: the neutral
    /// fibre's distance from the inner surface over the thickness.
    pub k_factor: Expr,
    pub flanges: Vec<Flange>,
    pub relief: Option<CornerRelief>,
    /// Sketches in the base sketch's plane whose closed profiles are holes
    /// and cut-outs of the flat (the base or a flange's) each lies wholly
    /// inside in the flat pattern; a sketch switched off (`when`,
    /// suppressed) cuts nothing. Each comes before this feature.
    #[serde(default)]
    pub cutouts: Vec<FeatureId>,
    /// How the corners where two flanged edges meet are closed (`None`:
    /// open) ...
    #[serde(default)]
    pub corner: Option<CornerClose>,
    /// ... but these, each its own.
    #[serde(default)]
    pub corners: Vec<SheetCorner>,
    /// The id the next new flange gets: above every id a flange of this
    /// part has had, so a deleted flange's id never comes back.
    #[serde(default)]
    pub next_flange_id: u32,
    /// The relief cut at every end of a bend that lies inside its edge (an
    /// inset flange's, a tab's): a notch of its shape `size` deep into the
    /// parent flat beside the bend's end and `size` along the edge
    /// (`None`: none, the edge turns into the bend's end).
    #[serde(default)]
    pub bend_relief: Option<CornerRelief>,
}

/// The highest id a flange may have: its faces are named `face(2 · id −
/// 1)` and `face(2 · id)`, which stay below 2³² (docs/design/sheetmetal.md
/// › *Names*).
pub const MAX_FLANGE_ID: u32 = (1 << 31) - 1;

impl SheetMetal {
    /// Why the flanges' ids are not sound, if they are not: one is 0 or
    /// above [`MAX_FLANGE_ID`], two are alike, one is not below
    /// `next_flange_id`, or that is past the highest id plus 1.
    pub fn flange_ids_fault(&self) -> Option<String> {
        if self.next_flange_id > MAX_FLANGE_ID + 1 {
            return Some(format!(
                "the next flange id {} is past the highest, {MAX_FLANGE_ID}, plus 1",
                self.next_flange_id
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, f) in self.flanges.iter().enumerate() {
            if f.id == 0 {
                return Some(format!("flange {i} has no id"));
            }
            if !seen.insert(f.id) {
                return Some(format!("flange {i}'s id {} is another flange's", f.id));
            }
            if f.id >= self.next_flange_id {
                return Some(format!(
                    "flange {i}'s id {} is not below the next id {}",
                    f.id, self.next_flange_id
                ));
            }
        }
        None
    }

    /// The expressions, with the name of their parameter
    /// (`flanges.2.angle`).
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out: Vec<(String, &Expr)> = vec![
            ("thickness".to_string(), &self.thickness),
            ("radius".to_string(), &self.radius),
            ("kFactor".to_string(), &self.k_factor),
        ];
        for (i, f) in self.flanges.iter().enumerate() {
            out.push((format!("flanges.{i}.length"), &f.length));
            out.push((format!("flanges.{i}.angle"), &f.angle));
            if let Some(r) = &f.radius {
                out.push((format!("flanges.{i}.radius"), r));
            }
            if let Some(k) = &f.k_factor {
                out.push((format!("flanges.{i}.kFactor"), k));
            }
            if let Some(e) = &f.inset_start {
                out.push((format!("flanges.{i}.insetStart"), e));
            }
            if let Some(e) = &f.inset_end {
                out.push((format!("flanges.{i}.insetEnd"), e));
            }
        }
        if let Some(relief) = &self.relief {
            if let Some(e) = relief.shape.expr() {
                out.push(("relief.shape".to_string(), e));
            }
            out.push(("relief.size".to_string(), &relief.size));
        }
        if let Some(relief) = &self.bend_relief {
            if let Some(e) = relief.shape.expr() {
                out.push(("bendRelief.shape".to_string(), e));
            }
            out.push(("bendRelief.size".to_string(), &relief.size));
        }
        if let Some(corner) = &self.corner {
            if let Some(e) = corner.kind.expr() {
                out.push(("corner.kind".to_string(), e));
            }
            if let Some(e) = &corner.gap {
                out.push(("corner.gap".to_string(), e));
            }
        }
        for (i, c) in self.corners.iter().enumerate() {
            if let Some(e) = c.kind.expr() {
                out.push((format!("corners.{i}.kind"), e));
            }
            if let Some(e) = &c.gap {
                out.push((format!("corners.{i}.gap"), e));
            }
        }
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out: Vec<&mut Expr> =
            vec![&mut self.thickness, &mut self.radius, &mut self.k_factor];
        for f in &mut self.flanges {
            out.push(&mut f.length);
            out.push(&mut f.angle);
            if let Some(r) = &mut f.radius {
                out.push(r);
            }
            if let Some(k) = &mut f.k_factor {
                out.push(k);
            }
            if let Some(e) = &mut f.inset_start {
                out.push(e);
            }
            if let Some(e) = &mut f.inset_end {
                out.push(e);
            }
        }
        if let Some(relief) = &mut self.relief {
            if let Some(e) = relief.shape.expr_mut() {
                out.push(e);
            }
            out.push(&mut relief.size);
        }
        if let Some(relief) = &mut self.bend_relief {
            if let Some(e) = relief.shape.expr_mut() {
                out.push(e);
            }
            out.push(&mut relief.size);
        }
        if let Some(corner) = &mut self.corner {
            if let Some(e) = corner.kind.expr_mut() {
                out.push(e);
            }
            if let Some(e) = &mut corner.gap {
                out.push(e);
            }
        }
        for c in &mut self.corners {
            if let Some(e) = c.kind.expr_mut() {
                out.push(e);
            }
            if let Some(e) = &mut c.gap {
                out.push(e);
            }
        }
        out
    }
}
