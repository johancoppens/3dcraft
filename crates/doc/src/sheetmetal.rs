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

/// The relief cut at every corner of the base where two flanged edges
/// meet: its shape and how far along each edge from the corner it reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CornerRelief {
    pub shape: ReliefShape,
    pub size: Expr,
}

/// A sheet-metal part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetMetal {
    /// The sketch whose one closed profile is the base flat (holes
    /// allowed); it has to come before this feature in the history.
    pub sketch: FeatureId,
    pub thickness: Expr,
    /// The inner radius of the bends that do not give their own.
    pub radius: Expr,
    /// The K-factor of the bends that do not give their own: the neutral
    /// fibre's distance from the inner surface over the thickness.
    pub k_factor: Expr,
    pub flanges: Vec<Flange>,
    pub relief: Option<CornerRelief>,
}

impl SheetMetal {
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
        }
        if let Some(relief) = &self.relief {
            out.push(("relief.size".to_string(), &relief.size));
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
        }
        if let Some(relief) = &mut self.relief {
            out.push(&mut relief.size);
        }
        out
    }
}
