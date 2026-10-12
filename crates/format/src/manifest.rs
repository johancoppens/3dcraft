//! The types of `manifest.json`, the file's own description of a document.
//!
//! They mirror `craft3d-doc` but are deliberately separate: the document
//! model is free to change shape, the file is not. What is written here is
//! what agents and humans read and diff, so every rename or reorder in this
//! module is a format change (bump [`crate::FORMAT_VERSION`] and add a
//! migration, AGENTS.md §8).
//!
//! Rules:
//! - every struct has `deny_unknown_fields`: a field this version does not
//!   know is an error on load, not something that silently disappears on
//!   the next save;
//! - `#[serde(default)]` only on fields that are optional by design, so a
//!   file that lost a required field is an error and not a default;
//! - fields are written in declaration order, so the order here is the
//!   order in the file.

use serde::{Deserialize, Serialize};

/// The value of the `format` field: tells a `.3dcraft` manifest from any
/// other JSON that happens to be called `manifest.json`.
pub const FORMAT_NAME: &str = "3dcraft";

/// The root of `manifest.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub document: Document,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub units: Units,
    pub tolerance: Tolerance,
    /// In the order they were defined.
    pub variables: Vec<Variable>,
    /// The design history, in order of application.
    pub features: Vec<Feature>,
    /// The id of the feature regeneration stops after; `null` or absent
    /// regenerates everything.
    #[serde(default)]
    pub rollback: Option<u64>,
    pub next_feature_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Units {
    pub length: LengthUnit,
    pub angle: AngleUnit,
}

/// Written as the unit's symbol in the expression language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    #[serde(rename = "um")]
    Micrometre,
    #[serde(rename = "mm")]
    Millimetre,
    #[serde(rename = "cm")]
    Centimetre,
    #[serde(rename = "m")]
    Metre,
    #[serde(rename = "in")]
    Inch,
    #[serde(rename = "ft")]
    Foot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngleUnit {
    #[serde(rename = "deg")]
    Degree,
    #[serde(rename = "rad")]
    Radian,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    /// In the document's length unit.
    pub linear: f64,
    /// In radians.
    pub angular: f64,
}

/// `expr` is the source text the user wrote, never an evaluated number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    pub expr: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: u64,
    pub name: String,
    /// Always written; absent means not suppressed.
    #[serde(default)]
    pub suppressed: bool,
    pub kind: FeatureKind,
}

/// How a primitive is turned: `axis` is the direction of its local z axis
/// (three plain numbers), `angle` the rotation about it. Optional in the
/// file: features written before orientations existed are upright.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Orientation {
    pub axis: [String; 3],
    pub angle: String,
}

impl Default for Orientation {
    /// The document model's default, so the two cannot drift apart.
    fn default() -> Self {
        let craft3d_doc::Orientation { axis, angle } = craft3d_doc::Orientation::default();
        Self {
            axis: axis.map(|e| e.0),
            angle: angle.0,
        }
    }
}

/// The parameters of a feature, tagged with `"type"`. Every parameter is an
/// expression, stored as its source text; triples are x, y, z.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum FeatureKind {
    Block {
        position: [String; 3],
        size: [String; 3],
        #[serde(default)]
        orientation: Orientation,
    },
    Cylinder {
        position: [String; 3],
        radius: String,
        height: String,
        #[serde(default)]
        orientation: Orientation,
    },
    Sphere {
        position: [String; 3],
        radius: String,
    },
    Cone {
        position: [String; 3],
        radius: String,
        height: String,
        #[serde(default)]
        orientation: Orientation,
    },
    Torus {
        position: [String; 3],
        major: String,
        minor: String,
        #[serde(default)]
        orientation: Orientation,
    },
    /// A 2D sketch. The coordinates of its points and the radii of its
    /// circles are the last solution, in the document's length unit: the
    /// solver starts from them, so a reopened sketch does not flip.
    Sketch {
        plane: SketchPlane,
        /// In the order they were drawn.
        entities: Vec<SketchEntity>,
        /// In the order they were added.
        constraints: Vec<SketchConstraint>,
        next_entity_id: u64,
        next_constraint_id: u64,
    },
}

/// What a sketch lies on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SketchPlane {
    /// A plane through the world origin, moved by `offset` (an expression)
    /// along its normal.
    World { plane: WorldPlane, offset: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldPlane {
    Xy,
    Yz,
    Zx,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchEntity {
    /// Unique within the sketch, never reused.
    pub id: u64,
    /// Always written; absent means not construction geometry.
    #[serde(default)]
    pub construction: bool,
    pub geometry: SketchGeometry,
}

/// Lines, circles and arcs refer to point entities by id.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SketchGeometry {
    Point {
        x: f64,
        y: f64,
    },
    Line {
        start: u64,
        end: u64,
    },
    Circle {
        center: u64,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end`.
    Arc {
        center: u64,
        start: u64,
        end: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchConstraint {
    /// Unique within the sketch, never reused.
    pub id: u64,
    pub kind: SketchConstraintKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceDirection {
    #[default]
    Direct,
    Horizontal,
    Vertical,
}

/// A constraint, tagged with `"type"`. The other fields are entity ids,
/// except `value` (an expression) and `direction`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SketchConstraintKind {
    Coincident {
        a: u64,
        b: u64,
    },
    PointOn {
        point: u64,
        curve: u64,
    },
    /// `b` is null for a line, a second point otherwise.
    Horizontal {
        a: u64,
        b: Option<u64>,
    },
    Vertical {
        a: u64,
        b: Option<u64>,
    },
    Parallel {
        a: u64,
        b: u64,
    },
    Perpendicular {
        a: u64,
        b: u64,
    },
    Tangent {
        a: u64,
        b: u64,
    },
    Equal {
        a: u64,
        b: u64,
    },
    Concentric {
        a: u64,
        b: u64,
    },
    Symmetric {
        a: u64,
        b: u64,
        about: u64,
    },
    Midpoint {
        point: u64,
        line: u64,
    },
    Fixed {
        point: u64,
    },
    /// `b` is null for the length of line `a`.
    Distance {
        a: u64,
        b: Option<u64>,
        #[serde(default)]
        direction: DistanceDirection,
        value: String,
    },
    Angle {
        a: u64,
        b: u64,
        value: String,
    },
    Radius {
        curve: u64,
        value: String,
    },
    Diameter {
        curve: u64,
        value: String,
    },
}
