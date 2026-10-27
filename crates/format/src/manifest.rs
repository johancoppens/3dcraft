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
    /// The appearance and the material given to bodies, by the id of the
    /// feature that makes each, in ascending order of that id. Absent when
    /// there are none (and in files before version 16).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<BodyProperties>,
    /// The appearance given to single faces, by the id of the feature
    /// whose body has the face and the face's name there, in ascending
    /// order of the two. Absent when there are none (and in files before
    /// version 26).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<FaceProperties>,
    /// Named sets: a query of `select.query` kept under a name, as JSON
    /// text, in the order they were defined. Absent when there are none
    /// (and in files before version 27).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<NamedSet>,
    /// The assertions the design is checked against, in the order they
    /// were added. Absent when there are none (and in files before
    /// version 28).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<Assertion>,
    /// The ids of the features hidden in the view, in ascending order:
    /// view data, which regeneration does not read. Absent when none is
    /// hidden (and in files before version 30).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<u64>,
    /// What a gallery shows of the document. Absent when every field is
    /// empty (and in files before version 32).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<DocInfo>,
    /// Tables of values that expressions look up (`belt[2].pitch`), in
    /// the order they were defined. Absent when there are none (and in
    /// files before version 33).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
    /// Outputs: read-only variables computed after regeneration, from a
    /// measure or from an expression, in the order they were defined.
    /// Absent when there are none (and in files before version 43).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Output>,
}

/// One output: `{"name": "T", "measure": {"measure": "levelFor", ...}}`,
/// what is measured as an assertion keeps it, or `{"name": "GM", "expr":
/// "KB + BM - KG"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

/// A table: `{"name": "belt", "key": "profile", "columns": [{"name":
/// "profile"}, {"name": "pitch", "unit": "mm"}], "rows": [["GT2", 2]]}`.
/// A cell is a number in its column's unit or a text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub name: String,
    pub key: String,
    pub columns: Vec<TableColumn>,
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableColumn {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TableCell {
    Number(f64),
    Text(String),
}

/// The document info: every field optional, absent when empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocInfo {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// An SPDX identifier.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub licence: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// One assertion: what is measured (the JSON object `assert.add` was
/// given, as an object) and what it is expected to be.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    pub name: String,
    pub measure: serde_json::Value,
    pub expect: Expect,
}

/// What an assertion expects: `is` for a yes-or-no measure, else `eq`
/// with an optional `tol`, else `min` and/or `max` (expressions).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

/// A query kept under a name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedSet {
    pub name: String,
    pub query: String,
}

/// What one face of the body of a feature looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceProperties {
    pub feature: u64,
    pub face: Name,
    pub appearance: Appearance,
}

/// What the body of one feature looks like and is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyProperties {
    pub feature: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<BodyAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Material>,
}

/// A body's [`Appearance`], whose `color` may also be three expressions
/// of the variables (red, green and blue, each a plain number from 0 to
/// 255; version 37).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyAppearance {
    pub color: BodyColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f64>,
}

/// `[200, 16, 46]`, or `["woods[wood].red", "120", "if(dark, 40, 90)"]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BodyColor {
    Fixed([u8; 3]),
    Exprs([String; 3]),
}

/// `color` is sRGB, three bytes. `metallic` and `roughness` lie between
/// 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub color: [u8; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f64>,
}

/// `density` is in kg/m³, whatever the document's length unit; with
/// `density_expr` (version 37), an expression of the variables that gives
/// a plain number, the density is its value times `density`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub name: String,
    pub density: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_expr: Option<String>,
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
    /// The cap on any entity's own tolerance, in the document's length
    /// unit; absent for the default (1000 times `linear`). Absent in files
    /// written before it existed, which then mean the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loosest: Option<f64>,
}

/// `expr` is the source text the user wrote, never an evaluated number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    pub expr: String,
    /// Set for a public parameter; absent otherwise (and in files before
    /// version 32).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<PublicParam>,
}

/// How a public parameter is shown and set. `min`, `max`, `step` and
/// `default` are expressions; `unit` a unit symbol (`mm`, `deg`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicParam {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub order: i32,
    pub kind: ParamKind,
    /// The labels of a choice, the value 1 first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// The table whose key column gives a choice's labels (version 33).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub default: String,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// What kind of value a public parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    Number,
    Integer,
    Boolean,
    Choice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: u64,
    pub name: String,
    /// Always written; absent means not suppressed.
    #[serde(default)]
    pub suppressed: bool,
    /// The condition that switches the feature on (an expression); absent
    /// means always on. Since format 34.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
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
        /// Patterns of seed entities, in the order they were made (format
        /// version 18). Absent means none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        patterns: Vec<SketchPattern>,
        /// The id the next pattern gets (format version 18). Absent means
        /// 1, and 1 is not written.
        #[serde(
            default = "first_pattern_id",
            skip_serializing_if = "is_first_pattern_id"
        )]
        next_pattern_id: u64,
        /// Where the image points lay in the stored solution (format
        /// version 20): the solver carries the sketch along from there
        /// when an edit moves an image. Absent means none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        image_places: Vec<ImagePlace>,
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
    /// A solid through the closed profiles of the sketches of `sections`,
    /// in order (format version 8).
    Loft {
        sections: Vec<LoftSection>,
        /// Straight between sections that follow each other. Absent
        /// means smooth; written only when set.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ruled: bool,
        /// The last section joins the first. Written only when set.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        closed: bool,
    },
    /// A solid the one closed profile of sketch `sketch` sweeps along the
    /// curves of sketch `path` (format version 10), or along the datum
    /// helix `path` (format version 13).
    Sweep { sketch: u64, path: u64 },
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
    /// A straight line to place other features by; it makes no body.
    DatumAxis { by: AxisBy },
    /// A point to place other features by; it makes no body.
    DatumPoint { at: PointRef },
    /// A helix about `axis` that a sweep can follow; it makes no body
    /// (format version 13). Written without `left_handed` when it is
    /// right-handed.
    DatumHelix {
        axis: AxisRef,
        start: HelixStart,
        pitch: String,
        extent: HelixExtent,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        left_handed: bool,
    },
    /// Edges of the body of an earlier feature replaced by faces. Each
    /// edge is the two faces it lies between, the first distance being
    /// measured along the first; the body is that of the feature the first
    /// name starts with. With `chain` each edge stands for the edges that
    /// continue it smoothly as well.
    Chamfer {
        edges: Vec<[Name; 2]>,
        size: ChamferSize,
        #[serde(default)]
        chain: bool,
    },
    /// Edges of the body of an earlier feature replaced by faces tangent
    /// to both their neighbours, with one radius. Each edge is the two
    /// faces it lies between; the body is that of the feature the first
    /// name starts with. With `chain` each edge stands for the edges that
    /// continue it smoothly as well.
    Fillet {
        edges: Vec<[Name; 2]>,
        radius: String,
        #[serde(default)]
        chain: bool,
    },
    /// Faces of the body of an earlier feature moved along their normals
    /// by `distance` (outward positive), the faces next to them
    /// following. The body is that of the feature the first name starts
    /// with.
    OffsetFace { faces: Vec<Name>, distance: String },
    /// Faces of the body of an earlier feature moved together by one
    /// rigid motion: turned by `angle` about the line through `pivot`
    /// along `axis`, then shifted by `translate`; the faces next to them
    /// follow. The body is that of the feature the first name starts
    /// with. Format version 16.
    MoveFace {
        faces: Vec<Name>,
        translate: [String; 3],
        pivot: [String; 3],
        axis: [String; 3],
        angle: String,
    },
    /// A solid of its own: faces of the body of an earlier feature given
    /// `thickness`, to `side` of them (`outside`, `inside` or `both`;
    /// written always). Format version 16.
    Thicken {
        faces: Vec<Name>,
        thickness: String,
        side: ThickenSide,
    },
    /// Faces of an earlier body given `thickness` to `side` of them, a
    /// solid of its own, with a joint per listed edge (docs/design/plates.md).
    Plate {
        faces: Vec<Name>,
        thickness: String,
        side: PlateSide,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        joints: Vec<PlateJoint>,
    },
    /// Faces of the body of an earlier feature tilted by `angle` about
    /// the plane `neutral`, the pull along its normal (into the body for
    /// a face), the other way when `reversed` (written only when set).
    /// Format version 16.
    Draft {
        faces: Vec<Name>,
        neutral: PlaneRef,
        angle: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reversed: bool,
    },
    /// Faces of the body of an earlier feature taken out, the body closed
    /// again by the faces round them. The body is that of the feature
    /// the first name starts with.
    DeleteFace { faces: Vec<Name> },
    /// An ISO metric thread on the cylindrical face `face`, from the flat
    /// face `start` along it for `length` (format version 13). `size` is
    /// the designation (`"M10"`), or `{"diameter": "d"}`, the nominal
    /// diameter as an expression of a length, or absent: the size whose
    /// nominal diameter the face has (version 14). `pitch` is absent for
    /// the coarse pitch of the size (version 14; version 13 always wrote
    /// it). Written without `left_handed` and `modeled` when they are
    /// not set. A thread is cosmetic unless `modeled` (version 14;
    /// version 13 wrote `cosmetic` and cut a thread that did not say it).
    Thread {
        face: Name,
        start: Name,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<ThreadSize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pitch: Option<String>,
        length: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        left_handed: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        modeled: bool,
    },
    /// The body of `source` made hollow, with walls of `thickness` behind
    /// every face except those of `open`, which are taken away.
    Shell {
        source: u64,
        thickness: String,
        #[serde(default)]
        open: Vec<Name>,
    },
    /// The closed profiles of the sketch `sketch` offset by `distance`
    /// (format version 19): positive grows the material. `corners` is
    /// `round` (the default, left out when written) or `sharp`.
    OffsetProfile {
        sketch: u64,
        distance: String,
        #[serde(default, skip_serializing_if = "OffsetCorners::is_round")]
        corners: OffsetCorners,
    },
    /// A sheet: the flat faces the closed profiles of sketch `sketch` (or
    /// of an offset of them) bound (format version 22).
    Fill { sketch: u64 },
    /// One lump of the body of feature `source` as a body of its own: the
    /// lump that holds a piece of the face `face`, or, when pieces of that
    /// face lie in several lumps, the one `number` counts to in the order
    /// of where the lumps lie in space (format version 24). `number` is
    /// written only when set.
    Split {
        source: u64,
        face: Name,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        number: Option<u64>,
    },
    /// The developable sheet between two rails (format version 39): each
    /// rail is `{"sketch": id}` (the one open chain of its curves) or
    /// `{"edges": [names]}` (edges of earlier bodies, end to end).
    Developable { a: Rail, b: Rail },
    /// Faces of an earlier body laid flat as one piece (format version
    /// 39), placed at `at` and turned by `angle` about +z.
    Unfold {
        faces: Vec<Name>,
        at: [String; 3],
        angle: String,
    },
    /// Solid or sheet number `solid` (in file order) of an imported STEP
    /// file whose bytes are the ZIP entry `blobs/<sha256>.step`; `file_name`
    /// is the name it had, for messages (format version 25).
    Import {
        file_name: String,
        solid: u32,
        sha256: String,
    },
    /// A sheet folded along a crease pattern: the closed profiles of the
    /// other lines of sketch `sketch` bound the sheet, `creases` are its
    /// lines by entity id, each a mountain, valley or cut with its fold
    /// angle (absent: solved), and `fixed` (absent: the largest facet) is
    /// a point in the facet that stays in place (format version 38).
    Fold {
        sketch: u64,
        creases: Vec<FoldCrease>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixed: Option<[String; 2]>,
    },
    /// A sheet-metal part: the one closed profile of sketch `sketch` is
    /// the base flat, `flanges` are bent from its straight edges and from
    /// the tips of earlier flanges, with the part's `thickness`, inner
    /// `radius` and `k_factor` unless a flange gives its own; `relief` is
    /// cut at every corner where two flanged edges meet (format version 41);
    /// the closed profiles of the sketches `cutouts` (and the base sketch's
    /// other profiles) are holes of the flats they lie in (version 42).
    SheetMetal {
        sketch: u64,
        thickness: String,
        radius: String,
        k_factor: String,
        flanges: Vec<SheetFlange>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relief: Option<SheetRelief>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cutouts: Vec<u64>,
        /// How the corners where two flanged edges meet are closed, and
        /// the corners closed in their own way (format version 44).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        corner: Option<SheetCornerClose>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        corners: Vec<SheetCorner>,
    },
}

/// A choice among names: one of them (`"tear"`), or an expression whose
/// value picks one, 1 for the first (`{"expr": "shape"}`, format version
/// 44).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pick<T> {
    Named(T),
    Expr(PickExpr),
}

/// The expression of a [`Pick`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickExpr {
    pub expr: String,
}

/// How a corner where two flanged edges meet is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CornerKind {
    Open,
    Butt,
    Overlap,
    Miter,
}

/// The part's corner treatment: its kind and the gap between the two
/// flats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetCornerClose {
    pub kind: Pick<CornerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
}

/// One corner's treatment, named by the two flanges that meet there;
/// `over` the flange that covers it (butt and overlap).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetCorner {
    pub flanges: [u32; 2],
    pub kind: Pick<CornerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<u32>,
}

/// A flange of a sheet-metal part: on the base's edge of line `base` (an
/// entity id) or on the tip of flange `tip` (its place in the list).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetFlange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<u32>,
    pub length: String,
    pub angle: String,
    pub direction: BendDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_factor: Option<String>,
    #[serde(default, skip_serializing_if = "FlangeFrom::is_tangent")]
    pub from: FlangeFrom,
}

/// Which way a bend turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BendDirection {
    Up,
    Down,
}

/// Where a flange's length is measured from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlangeFrom {
    #[default]
    Tangent,
    Mould,
}

impl FlangeFrom {
    fn is_tangent(&self) -> bool {
        *self == FlangeFrom::Tangent
    }
}

/// A corner relief of a sheet-metal part: `shape` is `rect`, `round` or
/// `tear` (format version 41), or an expression that picks one, 1 to 3
/// (version 44).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetRelief {
    pub shape: Pick<ReliefShape>,
    pub size: String,
}

/// The shape of a corner relief.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReliefShape {
    Rect,
    Round,
    Tear,
}

/// A crease of a fold: a line of the sketch by entity id, `mountain`,
/// `valley` or `cut`, and its fold angle when it is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldCrease {
    pub line: u64,
    pub kind: CreaseKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<String>,
}

/// How a crease folds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreaseKind {
    Mountain,
    Valley,
    Cut,
}

/// A rail of a developable (format version 39).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rail {
    Sketch(u64),
    Edges(Vec<Name>),
}

/// How an offset profile goes round a corner where its curves part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OffsetCorners {
    #[default]
    Round,
    Sharp,
}

impl OffsetCorners {
    fn is_round(&self) -> bool {
        *self == OffsetCorners::Round
    }
}

/// Where the layer of a `thicken` lies against its faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThickenSide {
    Outside,
    Inside,
    Both,
}

/// Where a plate's material lies against its faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlateSide {
    In,
    Out,
    Mid,
}

/// How a plate ends along an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JointKind {
    Square,
    Bevel,
    Mitre,
    Touch,
    Over,
    Under,
    Onto,
}

/// The joint of a plate along one edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlateJoint {
    pub edge: Name,
    pub kind: JointKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<Name>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<PlateSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onto: Option<PlaneRef>,
}

/// The size of a thread: a designation of ISO 261 (`"M10"`), or the
/// nominal diameter as an expression of a length (`{"diameter": "d"}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThreadSize {
    Designation(String),
    Diameter(ThreadDiameter),
}

/// A thread's nominal diameter, an expression of a length.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadDiameter {
    pub diameter: String,
}

/// How far a chamfer cuts the two faces of an edge back: expressions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ChamferSize {
    Equal { distance: String },
    TwoDistances { distance: String, second: String },
    DistanceAngle { distance: String, angle: String },
}

/// How a datum axis is placed: through two points, along the straight
/// edge between two faces, the axis of a cylinder, cone or torus face,
/// or the line two planes meet in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum AxisBy {
    TwoPoints { points: [PointRef; 2] },
    Edge { between: [Name; 2] },
    Face { face: Name },
    TwoPlanes { planes: [PlaneRef; 2] },
}

/// Where a helix starts: at a radius from its axis, or at a point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelixStart {
    Radius { radius: String },
    Point { point: PointRef },
}

/// How far a helix goes: a number of turns, or a height along its axis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelixExtent {
    Turns { turns: String },
    Height { height: String },
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
/// the straight edge between two faces of one body, or a datum axis by
/// the id of its feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum AxisRef {
    World { axis: WorldAxis },
    Edge { between: [Name; 2] },
    Datum { feature: u64 },
}

/// A point a datum is placed relative to: coordinates (three
/// expressions), a vertex by its persistent name, the centre of the
/// circular edge between two faces of one body, or a datum point by the
/// id of its feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PointRef {
    At { position: [String; 3] },
    Vertex { vertex: Name },
    Center { between: [Name; 2] },
    Datum { feature: u64 },
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

/// One profile of a loft: the sketch by the id of its feature, and the
/// curve of it the loop starts with (an entity id), when one is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoftSection {
    pub sketch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
}

/// The axis of a revolution: the sketch's x or y axis, one of its lines
/// by entity id, or a datum axis by the id of its feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum RevolveAxis {
    X,
    Y,
    Line { entity: u64 },
    Datum { axis: u64 },
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
        /// The id of a datum axis to turn about instead; `point` and
        /// `axis` are then kept, unused. Written only when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<u64>,
        /// A screw: the length each instance is moved along the axis
        /// beyond its turn (an expression). Written only when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<String>,
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
    /// For a `between` (an edge) and a `corner` (a vertex) that lists
    /// any: the faces it is named by. Absent for every other role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<Box<NameFaces>>,
}

/// The faces an edge or a vertex is named by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameFaces {
    /// The names of the faces of the feature's body it lies on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<Name>,
    /// When several lie on the same faces: the names of the faces near it
    /// that tell it apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Name>,
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
    /// One of several entities the name in `of` would name at once (the
    /// faces a spline sweeps when it is cut into pieces or turned a full
    /// turn); `sources` is its number, from 1.
    Piece,
    /// An entity of a primitive; `sources` is its number.
    Face,
    Edge,
    Vertex,
    /// An edge by the faces it lies between (`faces`); `sources` is empty
    /// or the one number that tells it from others between the same faces.
    Between,
    /// A vertex by the faces that meet in it (`faces`).
    Corner,
    /// The face a chamfer puts in the place of an edge; `sources` is the
    /// place of the edge in the chamfer's list and, when that reference
    /// gave several edges, which of them.
    Chamfer,
    /// The flat face where three chamfers meet; `sources` are the places
    /// of the three edges in the chamfer's list.
    ChamferCorner,
    /// The face a fillet puts in the place of an edge; `sources` as for
    /// `chamfer`.
    Fillet,
    /// The piece of a ball where three fillets meet; `sources` as for
    /// `chamferCorner`.
    FilletCorner,
    /// A face of the groove of a thread; `sources` is one plain number:
    /// 0 the root, 1 and 2 the flanks (format version 13).
    Thread,
    /// The face a plate ends on along an edge of its sheet; `sources` is
    /// one plain number (the hash of the edge's name), followed by the
    /// sheet's face.
    Rim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldPlane {
    Xy,
    Yz,
    Zx,
}

/// Where an image point lay in the stored solution of its sketch, and the
/// numbers of the map that put it there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePlace {
    pub entity: u64,
    pub at: [f64; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<PlaceMap>,
}

/// `{"type": "shift", "offset": [x, y]}`, `{"type": "chain", "k": 2}` or
/// `{"type": "turn", "angle": 1.047, "at": [x, y]}` (radians).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PlaceMap {
    Shift { offset: [f64; 2] },
    Chain { k: f64 },
    Turn { angle: f64, at: [f64; 2] },
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

fn first_pattern_id() -> u64 {
    1
}

fn is_first_pattern_id(id: &u64) -> bool {
    *id == 1
}

/// A pattern inside a sketch (format version 18): its seed entities, the
/// number of instances (an expression) and how they lie. The instances
/// are not stored; they are made from the solved seed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchPattern {
    /// Unique within the sketch, never reused.
    pub id: u64,
    pub seed: Vec<u64>,
    pub count: String,
    pub layout: SketchPatternLayout,
    /// The patterns repeated whole as part of the seed (format version
    /// 21): a pattern of a pattern. Absent means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<u64>,
    /// The instances of the pattern's own level left out (format version
    /// 21), in ascending order. Absent means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<InstanceIndex>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SketchPatternLayout {
    /// Instance `k` moved by `k * step`; a grid adds rows by `second`.
    Linear {
        step: [String; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        second: Option<SecondDirection>,
    },
    /// A run of contour from point `from` to point `to`, repeated end to
    /// start.
    Chain { from: u64, to: u64 },
    /// Instance `k` turned by `k * angle` about `center`; with `chain` a
    /// ring of runs from the first point to the second.
    Circular {
        center: CenterRef,
        angle: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chain: Option<[u64; 2]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondDirection {
    pub step: [String; 2],
    pub count: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum CenterRef {
    Point { point: u64 },
    At { at: [String; 2] },
}

/// Which instance an image point is in: `{"at": 3}`, `{"grid": [2, 4]}`
/// or `"last"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum InstanceIndex {
    At(u32),
    Grid([u32; 2]),
    Last,
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
    /// The image of seed point `of` in instance `index` of pattern
    /// `pattern` (format version 18): no coordinates of its own. From
    /// version 21 `of` may be an image point of a pattern in `pattern`'s
    /// seed (an image of an image: `e8[last][last]`).
    Image {
        of: u64,
        pattern: u64,
        index: InstanceIndex,
    },
    /// A curve given by formulas (format version 35): the point (`x`(t),
    /// `y`(t)) for `t` from `from` to `to`, ending on the points `start`
    /// and `end` (one point for a closed curve). Fitted at every
    /// regeneration; nothing of the fit is stored.
    Formula {
        x: String,
        y: String,
        from: String,
        to: String,
        start: u64,
        end: u64,
    },
    /// A B-spline through the points `through`, fitted again at every
    /// regeneration (format version 35).
    Interpolated {
        degree: u32,
        through: Vec<u64>,
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
    /// The point's signed x coordinate in the sketch (format 29).
    X {
        point: u64,
        value: String,
    },
    /// The point's signed y coordinate in the sketch (format 29).
    Y {
        point: u64,
        value: String,
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
