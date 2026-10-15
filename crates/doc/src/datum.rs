//! Reference geometry: planes, axes and points that are features of the
//! history.
//!
//! A datum makes no body. It is a plane, a line or a point placed relative to something
//! else, and that something is referred to the way every feature refers
//! to another: a base plane by its name, a datum by the id of its
//! feature, a face of a body by its persistent name, an edge by the two
//! faces it lies between (docs/design/names.md), a vertex by its
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
    /// A datum axis made earlier in the history.
    Datum { feature: FeatureId },
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
    /// The centre of the circular edge two faces of one body meet in (the
    /// rim of a cylinder: between its wall and its top), by the names of
    /// the two faces.
    Center { between: [PersistentName; 2] },
    /// A datum point made earlier in the history.
    Datum { feature: FeatureId },
}

/// A point as a feature: something a datum plane or axis goes through.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatumPoint {
    pub at: PointRef,
}

impl DatumPoint {
    /// Every persistent name the point refers to, in a fixed order.
    pub fn names(&self) -> Vec<&PersistentName> {
        self.at.names()
    }

    /// Every feature the point is placed relative to, in a fixed order.
    pub fn inputs(&self) -> Vec<FeatureId> {
        self.at.inputs()
    }

    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        self.at.exprs("at")
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        self.at.exprs_mut()
    }
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

/// How a datum axis is placed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AxisBy {
    /// Through two different points, from the first to the second.
    TwoPoints { points: [PointRef; 2] },
    /// Along the straight edge two faces of one body meet in; see
    /// [`AxisRef::Edge`] for which way it points.
    Edge { between: [PersistentName; 2] },
    /// The axis of a face that has one: a cylinder, a cone or a torus,
    /// by the face's persistent name. It points the way the surface's own
    /// axis does: for a primitive along its `axis`, for a face swept from
    /// a circle along the sketch's normal, for a revolved face along the
    /// revolution's axis.
    Face { face: PersistentName },
    /// The line two planes meet in, along the cross product of their
    /// normals in the order given. The planes must not be parallel.
    TwoPlanes { planes: [PlaneRef; 2] },
}

impl AxisBy {
    /// The definition's name, as in command results.
    pub fn type_name(&self) -> &'static str {
        match self {
            AxisBy::TwoPoints { .. } => "twoPoints",
            AxisBy::Edge { .. } => "edge",
            AxisBy::Face { .. } => "face",
            AxisBy::TwoPlanes { .. } => "twoPlanes",
        }
    }
}

/// A straight line as a feature: something to revolve and to pattern
/// about, and to turn a datum plane about. It has a direction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatumAxis {
    pub by: AxisBy,
}

impl DatumAxis {
    /// Every persistent name the axis refers to, in a fixed order.
    pub fn names(&self) -> Vec<&PersistentName> {
        match &self.by {
            AxisBy::TwoPoints { points } => points.iter().flat_map(PointRef::names).collect(),
            AxisBy::Edge { between } => between.iter().collect(),
            AxisBy::Face { face } => vec![face],
            AxisBy::TwoPlanes { planes } => planes.iter().flat_map(PlaneRef::names).collect(),
        }
    }

    /// Every feature the axis is placed relative to, in a fixed order.
    pub fn inputs(&self) -> Vec<FeatureId> {
        match &self.by {
            AxisBy::TwoPoints { points } => points.iter().flat_map(PointRef::inputs).collect(),
            AxisBy::Edge { between } => between.iter().map(|face| face.feature).collect(),
            AxisBy::Face { face } => vec![face.feature],
            AxisBy::TwoPlanes { planes } => planes.iter().flat_map(PlaneRef::inputs).collect(),
        }
    }

    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        match &self.by {
            AxisBy::TwoPoints { points } => points
                .iter()
                .enumerate()
                .flat_map(|(i, point)| point.exprs(&format!("through.{i}")))
                .collect(),
            AxisBy::Edge { .. } | AxisBy::Face { .. } | AxisBy::TwoPlanes { .. } => Vec::new(),
        }
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match &mut self.by {
            AxisBy::TwoPoints { points } => {
                points.iter_mut().flat_map(PointRef::exprs_mut).collect()
            }
            AxisBy::Edge { .. } | AxisBy::Face { .. } | AxisBy::TwoPlanes { .. } => Vec::new(),
        }
    }
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
            AxisRef::World { .. } | AxisRef::Datum { .. } => Vec::new(),
            AxisRef::Edge { between } => between.iter().collect(),
        }
    }

    pub(crate) fn inputs(&self) -> Vec<FeatureId> {
        match self {
            AxisRef::World { .. } => Vec::new(),
            AxisRef::Edge { between } => between.iter().map(|face| face.feature).collect(),
            AxisRef::Datum { feature } => vec![*feature],
        }
    }
}

impl PointRef {
    pub(crate) fn names(&self) -> Vec<&PersistentName> {
        match self {
            PointRef::At { .. } | PointRef::Datum { .. } => Vec::new(),
            PointRef::Vertex { vertex } => vec![vertex],
            PointRef::Center { between } => between.iter().collect(),
        }
    }

    pub(crate) fn inputs(&self) -> Vec<FeatureId> {
        match self {
            PointRef::At { .. } => Vec::new(),
            PointRef::Vertex { vertex } => vec![vertex.feature],
            PointRef::Center { between } => between.iter().map(|face| face.feature).collect(),
            PointRef::Datum { feature } => vec![*feature],
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
            PointRef::Vertex { .. } | PointRef::Center { .. } | PointRef::Datum { .. } => {
                Vec::new()
            }
        }
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            PointRef::At { position } => position.iter_mut().collect(),
            PointRef::Vertex { .. } | PointRef::Center { .. } | PointRef::Datum { .. } => {
                Vec::new()
            }
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

/// Where a helix starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HelixStart {
    /// At this distance from the axis, level with the axis's own point,
    /// towards the first of the world's x, y and z axes that is the least
    /// along the axis (x for an upright axis).
    Radius { radius: Expr },
    /// At a point off the axis: its distance from the axis is the radius,
    /// and the helix starts level with it.
    Point { point: PointRef },
}

/// How far a helix goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HelixExtent {
    /// So many turns, a number that need not be whole.
    Turns { turns: Expr },
    /// So far along the axis.
    Height { height: Expr },
}

/// A helix as a feature: a curve about an axis that a sweep can follow
/// (a coil spring, the groove of a thread). It advances one pitch along
/// the axis per turn, counter-clockwise seen from the tip of the axis
/// unless it is left-handed. It makes no body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatumHelix {
    pub axis: AxisRef,
    pub start: HelixStart,
    pub pitch: Expr,
    pub extent: HelixExtent,
    #[serde(default)]
    pub left_handed: bool,
}

impl DatumHelix {
    /// Every persistent name the helix refers to, in a fixed order.
    pub fn names(&self) -> Vec<&PersistentName> {
        let mut out = self.axis.names();
        if let HelixStart::Point { point } = &self.start {
            out.extend(point.names());
        }
        out
    }

    /// Every feature the helix is placed relative to, in a fixed order.
    pub fn inputs(&self) -> Vec<FeatureId> {
        let mut out = self.axis.inputs();
        if let HelixStart::Point { point } = &self.start {
            out.extend(point.inputs());
        }
        out
    }

    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out = match &self.start {
            HelixStart::Radius { radius } => vec![("radius".to_string(), radius)],
            HelixStart::Point { point } => point.exprs("start"),
        };
        out.push(("pitch".to_string(), &self.pitch));
        out.push(match &self.extent {
            HelixExtent::Turns { turns } => ("turns".to_string(), turns),
            HelixExtent::Height { height } => ("height".to_string(), height),
        });
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = match &mut self.start {
            HelixStart::Radius { radius } => vec![radius],
            HelixStart::Point { point } => point.exprs_mut(),
        };
        out.push(&mut self.pitch);
        out.push(match &mut self.extent {
            HelixExtent::Turns { turns } => turns,
            HelixExtent::Height { height } => height,
        });
        out
    }
}
