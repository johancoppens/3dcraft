//! Reference geometry: planes that are features of the history.
//!
//! A datum makes no body. It is a plane placed relative to something
//! else, and that something is referred to the way every feature refers
//! to another: a base plane by its name, a datum by the id of its
//! feature, a face of a body by its persistent name, an edge by the two
//! faces it lies between (docs/architecture.md §4.1), a vertex by its
//! persistent name. Where the datum lies is derived by regeneration and
//! never stored.

use crate::{Expr, FeatureId, PersistentName, WorldPlane};
use serde::{Deserialize, Serialize};

/// An axis of the world, through its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldAxis {
    X,
    Y,
    Z,
}

impl WorldAxis {
    pub const ALL: [WorldAxis; 3] = [WorldAxis::X, WorldAxis::Y, WorldAxis::Z];

    pub fn name(self) -> &'static str {
        match self {
            WorldAxis::X => "x",
            WorldAxis::Y => "y",
            WorldAxis::Z => "z",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// A plane something is placed relative to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlaneRef {
    /// A base plane, through the world origin.
    World { plane: WorldPlane },
    /// A flat face of a body made earlier in the history, by its
    /// persistent name; its normal points out of the body.
    Face { face: PersistentName },
    /// A datum plane made earlier in the history.
    Datum { feature: FeatureId },
}

/// A straight line something is placed relative to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AxisRef {
    /// An axis of the world, through its origin.
    World { axis: WorldAxis },
    /// The straight edge two faces of one body meet in, by the names of
    /// the two faces. When both faces are flat the line points along the
    /// cross product of their outward normals, in the order given.
    Edge { between: [PersistentName; 2] },
}

/// A point something is placed relative to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PointRef {
    /// At coordinates in the world: three lengths.
    At { position: [Expr; 3] },
    /// A vertex of a body made earlier in the history, by its persistent
    /// name.
    Vertex { vertex: PersistentName },
}

/// How a datum plane is placed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlaneBy {
    /// `from`, moved by `distance` along its normal. Origin and axes are
    /// those of `from`.
    Offset { from: PlaneRef, distance: Expr },
    /// `from`, turned by `angle` about `about` (counter-clockwise seen
    /// from the tip of the axis) and passing through that axis. The axis
    /// has to be parallel to `from`.
    Angle {
        from: PlaneRef,
        about: AxisRef,
        angle: Expr,
    },
    /// Through three points that are not on one line. The origin is the
    /// first point, x points at the second, and the normal is such that
    /// the three go round it counter-clockwise.
    ThreePoints { points: [PointRef; 3] },
}

impl PlaneBy {
    /// The definition's name, as in command results.
    pub fn type_name(&self) -> &'static str {
        match self {
            PlaneBy::Offset { .. } => "offset",
            PlaneBy::Angle { .. } => "angle",
            PlaneBy::ThreePoints { .. } => "threePoints",
        }
    }
}

/// A plane as a feature: something a sketch can lie on and a body can be
/// mirrored in. It has an origin and axes, like a base plane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatumPlane {
    pub by: PlaneBy,
}

impl PlaneRef {
    pub(crate) fn names(&self) -> Vec<&PersistentName> {
        match self {
            PlaneRef::Face { face } => vec![face],
            PlaneRef::World { .. } | PlaneRef::Datum { .. } => Vec::new(),
        }
    }

    pub(crate) fn inputs(&self) -> Vec<FeatureId> {
        match self {
            PlaneRef::World { .. } => Vec::new(),
            PlaneRef::Face { face } => vec![face.feature],
            PlaneRef::Datum { feature } => vec![*feature],
        }
    }
}

impl AxisRef {
    pub(crate) fn names(&self) -> Vec<&PersistentName> {
        match self {
            AxisRef::World { .. } => Vec::new(),
            AxisRef::Edge { between } => between.iter().collect(),
        }
    }

    pub(crate) fn inputs(&self) -> Vec<FeatureId> {
        match self {
            AxisRef::World { .. } => Vec::new(),
            AxisRef::Edge { between } => between.iter().map(|face| face.feature).collect(),
        }
    }
}

impl PointRef {
    pub(crate) fn names(&self) -> Vec<&PersistentName> {
        match self {
            PointRef::At { .. } => Vec::new(),
            PointRef::Vertex { vertex } => vec![vertex],
        }
    }

    pub(crate) fn inputs(&self) -> Vec<FeatureId> {
        match self {
            PointRef::At { .. } => Vec::new(),
            PointRef::Vertex { vertex } => vec![vertex.feature],
        }
    }

    /// The expressions of the point, named `<name>.x` and so on.
    pub(crate) fn exprs(&self, name: &str) -> Vec<(String, &Expr)> {
        match self {
            PointRef::At { position } => position
                .iter()
                .zip(["x", "y", "z"])
                .map(|(e, axis)| (format!("{name}.{axis}"), e))
                .collect(),
            PointRef::Vertex { .. } => Vec::new(),
        }
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            PointRef::At { position } => position.iter_mut().collect(),
            PointRef::Vertex { .. } => Vec::new(),
        }
    }
}

impl DatumPlane {
    /// Every persistent name the plane refers to, in a fixed order.
    pub fn names(&self) -> Vec<&PersistentName> {
        match &self.by {
            PlaneBy::Offset { from, .. } => from.names(),
            PlaneBy::Angle { from, about, .. } => {
                let mut out = from.names();
                out.extend(about.names());
                out
            }
            PlaneBy::ThreePoints { points } => points.iter().flat_map(PointRef::names).collect(),
        }
    }

    /// Every feature the plane is placed relative to, in a fixed order.
    pub fn inputs(&self) -> Vec<FeatureId> {
        match &self.by {
            PlaneBy::Offset { from, .. } => from.inputs(),
            PlaneBy::Angle { from, about, .. } => {
                let mut out = from.inputs();
                out.extend(about.inputs());
                out
            }
            PlaneBy::ThreePoints { points } => points.iter().flat_map(PointRef::inputs).collect(),
        }
    }

    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        match &self.by {
            PlaneBy::Offset { distance, .. } => vec![("distance".to_string(), distance)],
            PlaneBy::Angle { angle, .. } => vec![("angle".to_string(), angle)],
            PlaneBy::ThreePoints { points } => points
                .iter()
                .enumerate()
                .flat_map(|(i, point)| point.exprs(&format!("through.{i}")))
                .collect(),
        }
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match &mut self.by {
            PlaneBy::Offset { distance, .. } => vec![distance],
            PlaneBy::Angle { angle, .. } => vec![angle],
            PlaneBy::ThreePoints { points } => {
                points.iter_mut().flat_map(PointRef::exprs_mut).collect()
            }
        }
    }
}
