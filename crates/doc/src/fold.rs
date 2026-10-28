//! A fold: a crease pattern on the flat sheet a sketch bounds, folded
//! rigidly by its fold angles (docs/design/origami.md).

use crate::{CurveIndex, CurveRef, EntityId, Expr, FeatureId};
use serde::{Deserialize, Serialize};

/// The most creases one fold may have.
pub const MAX_FOLD_CREASES: usize = 4096;

/// How a crease folds, seen from the side the sketch's normal points to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreaseKind {
    /// The paper folds away from the viewer: the crease rises.
    Mountain,
    /// The paper folds towards the viewer: the crease sinks.
    Valley,
    /// The sheet is cut through along the line.
    Cut,
}

impl CreaseKind {
    pub fn name(self) -> &'static str {
        match self {
            CreaseKind::Mountain => "mountain",
            CreaseKind::Valley => "valley",
            CreaseKind::Cut => "cut",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mountain" => Some(CreaseKind::Mountain),
            "valley" => Some(CreaseKind::Valley),
            "cut" => Some(CreaseKind::Cut),
            _ => None,
        }
    }
}

/// A crease of a fold: a straight line of the sketch (drawn or
/// construction), how it folds and by how much.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crease {
    pub line: EntityId,
    pub kind: CreaseKind,
    /// The fold angle, from 0 (flat) to 180 degrees (folded flat); which
    /// way is the kind's. Every piece of the line folds by it. `None`:
    /// each piece's angle is solved from the others. A cut has none.
    pub angle: Option<Expr>,
    /// Which copy of the line the sketch's patterns made: the line itself
    /// (`Seed`, `e7`), one instance (`e7[2]`, `e7[1,3]`), or the line and
    /// every instance of it (`All`, `e7[*]`). A crease that names one
    /// instance wins over `e7[*]` for that instance.
    #[serde(default)]
    pub index: CurveIndex,
    /// With `All`: the kind alternates along the instances, the given one
    /// on the line itself and every instance whose indices add up to an
    /// even number, the other one (mountain for valley) on the rest. A cut
    /// does not alternate.
    #[serde(default)]
    pub alternate: bool,
}

impl Crease {
    /// The line as the sketch names it: `e7`, `e7[2]`, `e7[*]`.
    pub fn name(&self) -> String {
        CurveRef {
            entity: self.line.0,
            index: self.index,
        }
        .to_string()
    }
}

/// A sheet folded along a crease pattern: the closed profiles of the
/// sketch's other lines bound the sheet, the creases divide it into
/// facets, and each facet turns as a rigid body about the creases it
/// shares with its neighbours.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fold {
    /// The sketch that holds the sheet's outline and the crease lines; it
    /// has to come before this feature in the history.
    pub sketch: FeatureId,
    pub creases: Vec<Crease>,
    /// A point of the sketch (in its axes) inside the facet that stays in
    /// place; `None`: the largest facet.
    pub fixed: Option<[Expr; 2]>,
    /// Thick panels instead of a sheet of no thickness: one prism per
    /// facet on a membrane at the sheet (docs/design/origami.md › *Thick
    /// panels*); `None`: the sheet.
    #[serde(default)]
    pub panels: Option<FoldPanels>,
}

/// Which side of the sheet thick panels lie on, as the sketch's normal
/// points (valleys fold towards the front).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelSide {
    Back,
    Front,
}

impl PanelSide {
    pub fn name(self) -> &'static str {
        match self {
            PanelSide::Back => "back",
            PanelSide::Front => "front",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "back" => Some(PanelSide::Back),
            "front" => Some(PanelSide::Front),
            _ => None,
        }
    }
}

/// The thick panels of a fold: their thickness and the clearance they
/// keep from every crease and cut (lengths), and their side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldPanels {
    pub thickness: Expr,
    pub clearance: Expr,
    pub side: PanelSide,
}

impl Fold {
    /// The fold angles that are given, with the name of their parameter
    /// (`crease.e7`).
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out: Vec<(String, &Expr)> = self
            .creases
            .iter()
            .filter_map(|c| Some((format!("crease.{}", c.name()), c.angle.as_ref()?)))
            .collect();
        if let Some([x, y]) = &self.fixed {
            out.push(("fixed.x".to_string(), x));
            out.push(("fixed.y".to_string(), y));
        }
        if let Some(p) = &self.panels {
            out.push(("panels.thickness".to_string(), &p.thickness));
            out.push(("panels.clearance".to_string(), &p.clearance));
        }
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out: Vec<&mut Expr> = self
            .creases
            .iter_mut()
            .filter_map(|c| c.angle.as_mut())
            .collect();
        if let Some([x, y]) = &mut self.fixed {
            out.push(x);
            out.push(y);
        }
        if let Some(p) = &mut self.panels {
            out.push(&mut p.thickness);
            out.push(&mut p.clearance);
        }
        out
    }
}
