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
    /// A solid swept from the closed profiles of a sketch along its
    /// normal. `sketch` is the id of the sketch feature.
    Extrude {
        sketch: u64,
        distance: String,
        /// Always written; absent means one-sided.
        #[serde(default)]
        symmetric: bool,
        /// Where the extrusion ends. Absent means at `distance`, and an
        /// extrusion that ends at its distance is written without it, so
        /// that a file that uses no other end is the file an earlier
        /// build wrote and reads.
        #[serde(default, skip_serializing_if = "ExtrudeEnd::is_distance")]
        end: ExtrudeEnd,
        /// Against the sketch's normal. Absent means along it; written
        /// only when set, for the same reason.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reversed: bool,
    },
    /// A solid swept by turning the closed profiles of sketch `sketch` by
    /// `angle` about `axis`, a line in the sketch's plane.
    Revolve {
        sketch: u64,
        axis: RevolveAxis,
        angle: String,
        /// Always written; absent means one-sided.
        #[serde(default)]
        symmetric: bool,
    },
    /// The body of feature `source`, turned by `angle` about the line
    /// through `pivot` along `axis` (three plain numbers), then shifted by
    /// `translate`. With `keep` the source keeps its own body.
    Transform {
        source: u64,
        translate: [String; 3],
        pivot: [String; 3],
        axis: [String; 3],
        angle: String,
        #[serde(default)]
        keep: bool,
    },
    /// The body of feature `source`, mirrored in the plane through `point`
    /// with normal `normal` (three plain numbers).
    Mirror {
        source: u64,
        point: [String; 3],
        normal: [String; 3],
        /// The id of a datum plane to mirror in instead; `point` and
        /// `normal` are then kept, unused. Written only when set, so a
        /// file without datums is the file an earlier build wrote.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plane: Option<u64>,
        #[serde(default)]
        keep: bool,
    },
    /// The bodies of features `target` and `tool` combined. With
    /// `keep_tool` the tool keeps its own body.
    Boolean {
        op: BooleanOp,
        target: u64,
        tool: u64,
        #[serde(default)]
        keep_tool: bool,
    },
    /// The body of feature `source`, `count` times in one body.
    Pattern {
        source: u64,
        count: String,
        layout: PatternLayout,
    },
    /// A plane to place other features by; it makes no body. `by` says
    /// how it is placed.
    DatumPlane { by: PlaneBy },
}

/// How a datum plane is placed: `from` moved by `distance` along its
/// normal; `from` turned by `angle` about `about`, through that axis; or
/// through three points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PlaneBy {
    Offset {
        from: PlaneRef,
        distance: String,
    },
    Angle {
        from: PlaneRef,
        about: AxisRef,
        angle: String,
    },
    ThreePoints {
        points: [PointRef; 3],
    },
}

/// A plane a datum is placed relative to: a base plane, a flat face by
/// its persistent name, or a datum plane by the id of its feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PlaneRef {
    World { plane: WorldPlane },
    Face { face: Name },
    Datum { feature: u64 },
}

/// A straight line a datum is placed relative to: an axis of the world,
/// or the straight edge between two faces of one body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum AxisRef {
    World { axis: WorldAxis },
    Edge { between: [Name; 2] },
}

/// A point a datum is placed relative to: coordinates (three
/// expressions), or a vertex by its persistent name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PointRef {
    At { position: [String; 3] },
    Vertex { vertex: Name },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
}

/// The axis of a revolution: the sketch's x or y axis, or one of its
/// lines by entity id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum RevolveAxis {
    X,
    Y,
    Line { entity: u64 },
}

/// Where the instances of a pattern go: instance `i` is the source moved
/// `i` steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PatternLayout {
    Linear {
        step: [String; 3],
    },
    Circular {
        point: [String; 3],
        axis: [String; 3],
        angle: String,
    },
}

/// What a sketch lies on. (Declared after [`FeatureKind`], which uses it.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SketchPlane {
    /// A plane through the world origin, moved by `offset` (an expression)
    /// along its normal.
    World { plane: WorldPlane, offset: String },
    /// A flat face of a body made earlier in the history, moved by
    /// `offset` along the face's outward normal.
    Face { face: Name, offset: String },
    /// A datum plane, by the id of its feature, moved by `offset` along
    /// its normal.
    Datum { plane: u64, offset: String },
}

/// Where an extrusion ends: at its `distance`, past everything built
/// before it, or on the surface of a face.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ExtrudeEnd {
    // With braces: serde lets a variant without them take any field.
    Distance {},
    ThroughAll {},
    UpToFace { face: Name },
}

impl Default for ExtrudeEnd {
    fn default() -> Self {
        ExtrudeEnd::Distance {}
    }
}

impl ExtrudeEnd {
    fn is_distance(&self) -> bool {
        matches!(self, ExtrudeEnd::Distance {})
    }
}

/// The persistent name of a face, edge or vertex: the feature that made
/// it, the part it plays, and the ids of the input entities it came from
/// (for an extrusion, sketch entity ids).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Name {
    pub feature: u64,
    pub role: NameRole,
    pub sources: Vec<u64>,
    /// For a `copy`: the name of the entity it is the image of. Absent for
    /// every other role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<Box<Name>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameRole {
    StartCap,
    EndCap,
    Side,
    StartEdge,
    EndEdge,
    LateralEdge,
    StartVertex,
    EndVertex,
    /// The image of an entity of another body; `sources` is the instance.
    Copy,
    /// An entity of a primitive; `sources` is its number.
    Face,
    Edge,
    Vertex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldPlane {
    Xy,
    Yz,
    Zx,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchEntity {
    /// Unique within the sketch, never reused.
    pub id: u64,
    /// Always written; absent means not construction geometry.
    #[serde(default)]
    pub construction: bool,
    pub geometry: SketchGeometry,
}

/// Lines, circles, arcs and splines refer to point entities by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// A polynomial B-spline on clamped uniform knots: its control points
    /// in order (format version 6).
    Spline {
        degree: u32,
        control: Vec<u64>,
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
