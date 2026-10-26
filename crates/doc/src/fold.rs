//! A fold: a crease pattern on the flat sheet a sketch bounds, folded
//! rigidly by its fold angles (docs/design/origami.md).

use crate::{EntityId, Expr, FeatureId};
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
}

impl Fold {
    /// The fold angles that are given, with the name of their parameter
    /// (`crease.e7`).
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out: Vec<(String, &Expr)> = self
            .creases
            .iter()
            .filter_map(|c| Some((format!("crease.{}", c.line), c.angle.as_ref()?)))
            .collect();
        if let Some([x, y]) = &self.fixed {
            out.push(("fixed.x".to_string(), x));
            out.push(("fixed.y".to_string(), y));
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
        out
    }
}
