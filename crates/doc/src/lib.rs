//! The document model: the recipe a model is built from.
//!
//! A document is variables and an ordered list of features. Bodies are not
//! stored here: they are derived by regeneration and can be thrown away
//! (AGENTS.md rule 2). Every numeric parameter is an [`Expr`], kept as the
//! text the user wrote. This crate is pure data with serde; it contains no
//! geometry.
//!
//! So far: variables, primitive features, sketches with their constraints
//! ([`sketch`]) and extrusions of sketches. The other feature kinds of
//! docs/design/document.md arrive with their milestones; `craft3d-format` fails to compile when a field is added
//! here without being saved (AGENTS.md §8).

pub mod appearance;
pub mod blob;
pub mod datum;
pub mod name;
pub mod sketch;

pub use appearance::{Appearance, BodyProperties, FaceProperties, MAX_FACE_APPEARANCES, Material};
pub use blob::{Blob, sha256};
pub use datum::{
    AxisBy, AxisRef, DatumAxis, DatumHelix, DatumPlane, DatumPoint, HelixExtent, HelixStart,
    PlaneBy, PlaneRef, PointRef, WorldAxis,
};
pub use name::{
    CurveIndex, CurvePath, CurveRef, FaceLists, MAX_CURVE_ENTITY, MAX_CURVE_INDEX,
    MAX_CURVE_LEVELS, MAX_NAME_DEPTH, MAX_NAME_FACES, MAX_NAME_SOURCES, NameRole, OffsetGroup,
    OffsetHead, OffsetKind, PersistentName, curve_groups, decode_continuation, offset_groups,
};

pub use sketch::{
    CenterRef, Constraint, ConstraintId, ConstraintKind, DistanceDirection, Entity, EntityId,
    Geometry, ImagePlace, InstanceIndex, MAX_ENTITY_ID, MAX_PATTERN_DEPTH, MAX_PATTERN_INSTANCES,
    MAX_SKETCH_PATTERNS, MAX_SPLINE_DEGREE, MAX_SPLINE_POINTS, PatternId, PlaceMap, Removal,
    SecondDirection, Sketch, SketchError, SketchPattern, SketchPatternLayout, SketchPlane,
    WorldPlane, clamped_uniform_knots, parse_image_path, parse_image_ref,
};

use craft3d_expr::{VariableError, VariableTable};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Largest number of features in a document.
pub const MAX_FEATURES: usize = 100_000;
/// Longest feature name, in bytes.
pub const MAX_NAME_LEN: usize = 256;

/// Identifies a feature for the lifetime of a document. Never reused, even
/// after the feature is deleted: persistent names will be built on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureId(pub u64);

impl fmt::Display for FeatureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "F{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LengthUnit {
    #[serde(rename = "um")]
    Micrometre,
    #[default]
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

impl LengthUnit {
    pub const ALL: [LengthUnit; 6] = [
        LengthUnit::Micrometre,
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Metre,
        LengthUnit::Inch,
        LengthUnit::Foot,
    ];

    /// The same unit in the expression language.
    pub fn as_expr_unit(self) -> craft3d_expr::Unit {
        use craft3d_expr::Unit;
        match self {
            LengthUnit::Micrometre => Unit::Micrometre,
            LengthUnit::Millimetre => Unit::Millimetre,
            LengthUnit::Centimetre => Unit::Centimetre,
            LengthUnit::Metre => Unit::Metre,
            LengthUnit::Inch => Unit::Inch,
            LengthUnit::Foot => Unit::Foot,
        }
    }

    pub fn symbol(self) -> &'static str {
        self.as_expr_unit().symbol()
    }

    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|u| u.symbol() == symbol)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AngleUnit {
    #[default]
    #[serde(rename = "deg")]
    Degree,
    #[serde(rename = "rad")]
    Radian,
}

/// The units a document's numbers are shown and modelled in. The kernel
/// works in the document's length unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Units {
    pub length: LengthUnit,
    pub angle: AngleUnit,
}

/// Modelling tolerance, as data (docs/design/plan.md).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    /// In the document's length unit.
    pub linear: f64,
    /// In radians.
    pub angular: f64,
    /// The cap on any vertex's or edge's own tolerance (tolerant entities,
    /// `craft3d_topo::Tolerances`), in the document's length unit; `None`
    /// for [`Tolerance::DEFAULT_LOOSEST`] times `linear`. Above it an
    /// operation refuses rather than let a tolerance grow unseen (the
    /// decision of 2026-10-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loosest: Option<f64>,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            linear: 1e-6,
            angular: 1e-9,
            loosest: None,
        }
    }
}

impl Tolerance {
    /// The cap on an entity's tolerance when the document sets none, as a
    /// multiple of the linear tolerance (`craft3d_math::DEFAULT_LOOSEST`).
    pub const DEFAULT_LOOSEST: f64 = craft3d_math::DEFAULT_LOOSEST;

    /// The cap on any entity's own tolerance: the document's, or
    /// [`Self::DEFAULT_LOOSEST`] times the linear tolerance.
    pub fn loosest_linear(&self) -> f64 {
        self.loosest.unwrap_or(Self::DEFAULT_LOOSEST * self.linear)
    }

    /// The least linear tolerance, in the document's length unit: 1e-9 of
    /// the smallest size the kernel is built and tested for (1e-3 units;
    /// AGENTS.md rule 3). Below it a tolerance is no longer about the
    /// geometry but about rounding (1e-300 was accepted until 2026-10-15).
    pub const LEAST_LINEAR: f64 = 1e-12;
    /// The greatest linear tolerance, in the document's length unit: 1e-3
    /// of the largest size the kernel is built and tested for (1e6 units).
    /// Coarser, it would merge what any model at those sizes keeps apart.
    pub const GREATEST_LINEAR: f64 = 1e3;

    /// Whether `linear` is a linear tolerance a document may have:
    /// finite and within [`Self::LEAST_LINEAR`]..=[`Self::GREATEST_LINEAR`].
    pub fn linear_in_range(linear: f64) -> bool {
        (Self::LEAST_LINEAR..=Self::GREATEST_LINEAR).contains(&linear)
    }

    /// This tolerance with a positive, finite linear part outside the
    /// range brought to the nearer end of it (what a file that was saved
    /// with one is read as). Anything else is kept, for
    /// [`Document::validate`] to refuse.
    pub fn in_range(self) -> Self {
        let linear = if self.linear.is_finite() && self.linear > 0.0 {
            self.linear.clamp(Self::LEAST_LINEAR, Self::GREATEST_LINEAR)
        } else {
            self.linear
        };
        Self { linear, ..self }
    }
}

/// An expression as the user wrote it, such as `"width / 2 + 3 mm"`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Expr(pub String);

impl Expr {
    pub fn new(source: impl Into<String>) -> Self {
        Self(source.into())
    }

    pub fn source(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub expr: Expr,
}

/// How a primitive is turned: its local z axis points along `axis`, and it
/// is then rotated by `angle` about that axis. The default leaves it along
/// the world axes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Orientation {
    /// Direction of the local z axis: three plain numbers, any length.
    pub axis: [Expr; 3],
    /// Rotation about `axis`, counter-clockwise seen from its tip.
    pub angle: Expr,
}

impl Default for Orientation {
    fn default() -> Self {
        Self {
            axis: [Expr::new("0"), Expr::new("0"), Expr::new("1")],
            angle: Expr::new("0 deg"),
        }
    }
}

/// A rectangular block with one corner at `position` and edges along its
/// local axes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub position: [Expr; 3],
    pub size: [Expr; 3],
    #[serde(default)]
    pub orientation: Orientation,
}

/// A cylinder along its local z axis, its base centred at `position`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cylinder {
    pub position: [Expr; 3],
    pub radius: Expr,
    pub height: Expr,
    #[serde(default)]
    pub orientation: Orientation,
}

/// A sphere centred at `position`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sphere {
    pub position: [Expr; 3],
    pub radius: Expr,
}

/// A cone along its local z axis: its base circle is centred at `position`
/// and its apex is `height` above it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cone {
    pub position: [Expr; 3],
    pub radius: Expr,
    pub height: Expr,
    #[serde(default)]
    pub orientation: Orientation,
}

/// A ring torus about its local z axis, centred at `position`: `major` is
/// the radius of the ring, `minor` that of the tube.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Torus {
    pub position: [Expr; 3],
    pub major: Expr,
    pub minor: Expr,
    #[serde(default)]
    pub orientation: Orientation,
}

/// A solid swept from the closed profiles of a sketch, along the sketch's
/// normal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extrude {
    /// The sketch feature the profiles come from. It has to come before
    /// this feature in the history; regeneration reports it when it does
    /// not, or is gone.
    pub sketch: FeatureId,
    /// How far, along the sketch's normal; a negative length goes the
    /// other way.
    pub distance: Expr,
    /// Half of `distance` to each side of the sketch plane. With
    /// [`ExtrudeEnd::ThroughAll`]: through everything on both sides.
    #[serde(default)]
    pub symmetric: bool,
    /// Where the extrusion ends. `distance` is only used by
    /// [`ExtrudeEnd::Distance`]; the other ends keep it, unused, so that
    /// going back to a distance brings the old one back.
    #[serde(default)]
    pub end: ExtrudeEnd,
    /// Against the sketch's normal instead of along it. (For a distance
    /// this is the same as negating it.)
    #[serde(default)]
    pub reversed: bool,
}

/// Where an extrusion ends.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtrudeEnd {
    /// `distance` from the sketch plane.
    #[default]
    Distance,
    /// Past everything the features before this one have built in the
    /// direction of the extrusion.
    ThroughAll,
    /// On the surface of a face of an earlier feature, by its persistent
    /// name: the end follows the face, flat or not.
    UpToFace { face: PersistentName },
}

impl ExtrudeEnd {
    /// The end's name, as in command parameters and results.
    pub fn type_name(&self) -> &'static str {
        match self {
            ExtrudeEnd::Distance => "distance",
            ExtrudeEnd::ThroughAll => "throughAll",
            ExtrudeEnd::UpToFace { .. } => "upToFace",
        }
    }
}

/// What a revolution turns about: a line in the plane of its sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RevolveAxis {
    /// The sketch's own x axis.
    X,
    /// The sketch's own y axis.
    Y,
    /// A line of the sketch (usually a construction line), from its start
    /// to its end.
    Line { entity: EntityId },
    /// A datum axis made earlier in the history; it has to lie in the
    /// sketch's plane.
    Datum { axis: FeatureId },
}

fn full_turn() -> Expr {
    Expr::new("360 deg")
}

/// A solid swept by turning the closed profiles of a sketch about an axis
/// in the sketch's plane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revolve {
    /// The sketch feature the profiles come from, as for [`Extrude`].
    pub sketch: FeatureId,
    pub axis: RevolveAxis,
    /// How far round, counter-clockwise seen from the tip of the axis; a
    /// negative angle goes the other way. A full turn closes the body.
    #[serde(default = "full_turn")]
    pub angle: Expr,
    /// Half of `angle` to each side of the sketch plane.
    #[serde(default)]
    pub symmetric: bool,
}

/// Most sections one loft has: what the kernel's loft takes
/// (`craft3d_kernel::loft::MAX_SECTIONS`, the most points its spline
/// through the sections interpolates; a test holds the two equal).
pub const MAX_LOFT_SECTIONS: usize = 256;

/// One profile of a loft: a sketch with one closed loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoftSection {
    pub sketch: FeatureId,
    /// The curve of the sketch the loop starts with, which is matched
    /// with the first curve of the other sections. `None` leaves it to
    /// the rule (the first section: its curve with the smallest id; the
    /// others: turned to lie nearest to the section before).
    #[serde(default)]
    pub start: Option<EntityId>,
}

/// A solid through a row of profiles in order: its side passes through
/// every one of them, its ends are the first and the last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loft {
    /// The profiles, in the order the loft passes through them.
    pub sections: Vec<LoftSection>,
    /// Straight lines between sections that follow each other, instead of
    /// a smooth surface through all of them.
    #[serde(default)]
    pub ruled: bool,
    /// The last section joins the first: a ring without ends.
    #[serde(default)]
    pub closed: bool,
}

/// How an offset outline goes round a corner where the offset curves of
/// its two neighbours part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OffsetCorners {
    /// An arc of the offset distance about the corner: the true offset
    /// (every point at the distance from the outline), the path a round
    /// beam or cutter takes.
    #[default]
    Round,
    /// The two neighbours carried on along their tangents to where they
    /// meet: a mitre.
    Sharp,
}

impl OffsetCorners {
    /// The name, as in command parameters and files.
    pub fn name(self) -> &'static str {
        match self {
            OffsetCorners::Round => "round",
            OffsetCorners::Sharp => "sharp",
        }
    }
}

/// The closed profiles of a sketch, every one of them, offset by a
/// distance: positive grows the material (outer loops out, holes in),
/// negative shrinks it. It makes no body; a feature that takes the
/// profiles of a sketch (an extrusion, a revolution, a sweep, a loft)
/// takes this feature in its place. It is made at regeneration from the
/// solved sketch with its patterns, so it follows every edit of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffsetProfile {
    /// The sketch feature whose profiles are offset.
    pub sketch: FeatureId,
    pub distance: Expr,
    #[serde(default)]
    pub corners: OffsetCorners,
}

/// The solid one closed profile sweeps out as it is carried along a path
/// in a plane, staying square to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sweep {
    /// The sketch feature whose one closed profile is carried.
    pub sketch: FeatureId,
    /// The sketch feature whose curves are the path: one chain of lines,
    /// arcs and splines, open or closed, or a circle.
    pub path: FeatureId,
}

fn zero3() -> [Expr; 3] {
    [Expr::new("0"), Expr::new("0"), Expr::new("0")]
}

fn z_axis() -> [Expr; 3] {
    [Expr::new("0"), Expr::new("0"), Expr::new("1")]
}

fn no_angle() -> Expr {
    Expr::new("0 deg")
}

/// The body of an earlier feature, moved: turned by `angle` about the line
/// through `pivot` along `axis`, then shifted by `translate`. The moved
/// body is this feature's; the source feature no longer shows one, unless
/// `keep` is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// The feature whose body is moved. It has to come before this
    /// feature in the history and have a body nothing else has taken.
    pub source: FeatureId,
    #[serde(default = "zero3")]
    pub translate: [Expr; 3],
    #[serde(default = "zero3")]
    pub pivot: [Expr; 3],
    /// Direction of the rotation axis: three plain numbers, any length.
    #[serde(default = "z_axis")]
    pub axis: [Expr; 3],
    /// Counter-clockwise seen from the tip of `axis`.
    #[serde(default = "no_angle")]
    pub angle: Expr,
    /// The source keeps its body: this feature makes a moved copy.
    #[serde(default)]
    pub keep: bool,
}

/// The body of an earlier feature, mirrored in the plane through `point`
/// with normal `normal`, or in a datum plane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirror {
    pub source: FeatureId,
    #[serde(default = "zero3")]
    pub point: [Expr; 3],
    /// Three plain numbers, any length.
    pub normal: [Expr; 3],
    /// A datum plane made earlier in the history to mirror in, instead of
    /// the plane through `point` with `normal`. Those two are kept,
    /// unused, so that going back to them brings the old plane back.
    #[serde(default)]
    pub plane: Option<FeatureId>,
    /// The source keeps its body: this feature makes a mirrored copy.
    #[serde(default)]
    pub keep: bool,
}

/// How the instances of a pattern are laid out. Instance 0 is the source
/// where it is; instance `i` is moved `i` steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PatternLayout {
    /// Each instance is shifted by `step` from the one before.
    Linear { step: [Expr; 3] },
    /// Each instance is turned by `angle` from the one before, about the
    /// line through `point` along `axis`.
    Circular {
        #[serde(default = "zero3")]
        point: [Expr; 3],
        #[serde(default = "z_axis")]
        axis: [Expr; 3],
        angle: Expr,
        /// A datum axis made earlier in the history to turn about,
        /// instead of the line through `point` along `axis`. Those two
        /// are kept, unused, so that going back to them brings the old
        /// line back.
        #[serde(default)]
        about: Option<FeatureId>,
        /// A screw: instance `i` is also moved by `i * step` along the
        /// axis it is turned about (a length; negative goes the other way
        /// along `axis`). The turn and the shift commute, as the shift is
        /// along the line turned about, so either order gives the same
        /// instance. `None` is a plain turn.
        #[serde(default)]
        step: Option<Expr>,
    },
}

/// The body of an earlier feature, repeated `count` times (the source
/// included) into one body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub source: FeatureId,
    /// A whole number of instances, at least 1: a plain number or a
    /// formula of plain numbers.
    pub count: Expr,
    pub layout: PatternLayout,
}

/// A sheet: the flat faces the closed profiles of a sketch bound, one
/// face per profile with its holes, each facing the way the sketch's
/// normal points. It has an area and no volume; a boolean trims it by a
/// solid, or cuts a solid with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    /// The sketch (or an offset of one's profiles) the profiles come
    /// from. It has to come before this feature in the history.
    pub sketch: FeatureId,
}

/// One lump of the body of an earlier feature as a body of its own: the
/// shell that bounds the lump from outside and the cavities in it
/// (`craft3d_kernel::lumps`). The body goes into this feature; the other
/// lumps are split features of their own with the same source, made
/// together by `body.split`, and the source's body is shared by all of
/// them.
///
/// Which lump is said by a face of the source's body, never by the order
/// of the shells (which the kernel's sewing sets, and an upstream edit
/// may change): the lump that holds a piece of the face `face`. When
/// pieces of that face lie in several lumps (a ring cut in two crescents:
/// both have pieces of the wall and of both cut faces), `number` says
/// which, by where the lumps lie in space (by x, then y, then z of the
/// centre of each lump's volume, coordinates within the tolerance
/// counting as one), as an edge or a vertex that no face tells from
/// another is numbered. `None` means the face lies in one lump only, and
/// the feature fails when an edit puts pieces of it in several; a number
/// fails when the face lies in one lump, or in fewer than it counts to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    pub source: FeatureId,
    pub face: PersistentName,
    #[serde(default)]
    pub number: Option<u64>,
}

/// A solid or a sheet read from a STEP file (ISO 10303-21) that the
/// document holds: the file's bytes, and which of its solids this is.
/// The document regenerates the body from the bytes, without the file
/// on disk (AGENTS.md rule 2); `craft3d-format` stores the bytes once per
/// content beside the manifest. `file_name` is the name the file had,
/// without its directories, for messages only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    pub file_name: String,
    /// The solid's position among the file's solids and sheets, in the
    /// order of their instances in the file (`craft3d-stepread`).
    pub solid: u32,
    pub data: Blob,
}

/// How two bodies are combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BooleanOp {
    /// Everything that is in either body.
    Union,
    /// The target without what the tool covers.
    Subtract,
    /// What is in both bodies.
    Intersect,
}

impl BooleanOp {
    pub const ALL: [BooleanOp; 3] = [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect];

    pub fn name(self) -> &'static str {
        match self {
            BooleanOp::Union => "union",
            BooleanOp::Subtract => "subtract",
            BooleanOp::Intersect => "intersect",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.name() == name)
    }
}

/// The bodies of two earlier features combined into one. Both bodies go
/// into this feature; with `keep_tool` the tool keeps its own body, to cut
/// several targets with one tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boolean {
    pub op: BooleanOp,
    pub target: FeatureId,
    pub tool: FeatureId,
    #[serde(default)]
    pub keep_tool: bool,
}

/// Most edge references one chamfer holds.
pub const MAX_CHAMFER_EDGES: usize = 1024;

/// How far a chamfer cuts back the two faces of an edge. "First" and
/// "second" are the two faces in the order the edge's reference gives
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChamferSize {
    /// The same distance along both faces.
    Equal { distance: Expr },
    /// `distance` along the first face, `second` along the second.
    TwoDistances { distance: Expr, second: Expr },
    /// `distance` along the first face; the chamfer face leaves that face
    /// at `angle`.
    DistanceAngle { distance: Expr, angle: Expr },
}

impl ChamferSize {
    /// The form's name, as in files and command results.
    pub fn type_name(&self) -> &'static str {
        match self {
            ChamferSize::Equal { .. } => "equal",
            ChamferSize::TwoDistances { .. } => "twoDistances",
            ChamferSize::DistanceAngle { .. } => "distanceAngle",
        }
    }
}

/// Edges of the body of an earlier feature replaced by faces: the two
/// faces next to each edge are cut back and a new face joins the cut
/// lines. The body goes into this feature.
///
/// An edge is referred to by the names of the two faces it lies between,
/// in the order that says which face the first distance is measured
/// along. Such a reference means every edge between those two faces (the
/// pieces a later cut divides an edge into stay chamfered). All faces are
/// faces of one body: the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chamfer {
    pub edges: Vec<[PersistentName; 2]>,
    pub size: ChamferSize,
    /// With each edge, the edges that continue it smoothly (the whole rim
    /// of a slot from one of its edges).
    #[serde(default)]
    pub chain: bool,
}

impl Chamfer {
    /// The feature whose body is chamfered: the one that made the first
    /// face referred to. `None` for a chamfer without edges.
    pub fn source(&self) -> Option<FeatureId> {
        self.edges.first().map(|[first, _]| first.feature)
    }
}

/// Most edge references one fillet holds.
pub const MAX_FILLET_EDGES: usize = 1024;

/// Edges of the body of an earlier feature replaced by faces that are
/// tangent to both faces next to the edge: what a ball of `radius`
/// touches as it rolls along the edge in contact with both. The body
/// goes into this feature.
///
/// An edge is referred to as a chamfer refers to it (see [`Chamfer`]):
/// by the names of the two faces it lies between. Their order says
/// nothing here, since a fillet treats both faces alike.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fillet {
    pub edges: Vec<[PersistentName; 2]>,
    pub radius: Expr,
    /// With each edge, the edges that continue it smoothly (the whole rim
    /// of a slot from one of its edges).
    #[serde(default)]
    pub chain: bool,
}

impl Fillet {
    /// The feature whose body is filleted: the one that made the first
    /// face referred to. `None` for a fillet without edges.
    pub fn source(&self) -> Option<FeatureId> {
        self.edges.first().map(|[first, _]| first.feature)
    }
}

/// Most faces one face operation (offset face, move face, thicken, draft,
/// delete face, the open faces of a shell) holds.
pub const MAX_FACE_OP_FACES: usize = 1024;

/// Faces of the body of an earlier feature moved along their normals by
/// `distance`: a flat face to the parallel plane, a cylinder, a cone or a
/// sphere grown or shrunk. Positive is outward, away from the material;
/// negative is into it. The faces next to them follow, so the body stays
/// closed. The body goes into this feature.
///
/// A face is referred to by its name. All faces are faces of one body:
/// the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffsetFace {
    pub faces: Vec<PersistentName>,
    pub distance: Expr,
}

impl OffsetFace {
    /// The feature whose body the faces are of: the one that made the
    /// first face referred to. `None` without faces.
    pub fn source(&self) -> Option<FeatureId> {
        self.faces.first().map(|first| first.feature)
    }
}

/// Faces of the body of an earlier feature moved together by one rigid
/// motion: turned by `angle` about the line through `pivot` along `axis`
/// (counter-clockwise seen from its tip), then shifted by `translate`, as
/// [`Transform`] moves a body. The faces next to them follow, so the body
/// stays closed. The body goes into this feature.
///
/// A face is referred to by its name. All faces are faces of one body:
/// the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveFace {
    pub faces: Vec<PersistentName>,
    #[serde(default = "zero3")]
    pub translate: [Expr; 3],
    #[serde(default = "zero3")]
    pub pivot: [Expr; 3],
    /// Direction of the rotation axis: three plain numbers, any length.
    #[serde(default = "z_axis")]
    pub axis: [Expr; 3],
    #[serde(default = "no_angle")]
    pub angle: Expr,
}

impl MoveFace {
    /// The feature whose body the faces are of: the one that made the
    /// first face referred to. `None` without faces.
    pub fn source(&self) -> Option<FeatureId> {
        self.faces.first().map(|first| first.feature)
    }
}

/// Faces of the body of an earlier feature tilted by `angle` about the
/// neutral plane, so that the part comes out of a mould along the pull
/// direction: the normal of `neutral` (for a face, into the body it
/// bounds), the other way when `reversed`. A drafted face keeps where it
/// meets the neutral plane and leans in by the angle as it goes along the
/// pull: a block drafted from its bottom face narrows toward its top.
/// The faces next to them follow. The body goes into this feature.
///
/// A face is referred to by its name. All faces are faces of one body:
/// the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub faces: Vec<PersistentName>,
    pub neutral: PlaneRef,
    pub angle: Expr,
    #[serde(default)]
    pub reversed: bool,
}

impl Draft {
    /// The feature whose body the faces are of: the one that made the
    /// first face referred to. `None` without faces.
    pub fn source(&self) -> Option<FeatureId> {
        self.faces.first().map(|first| first.feature)
    }
}

/// Where the layer of a [`Thicken`] lies against its faces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThickenSide {
    /// Outside the body: on the faces, away from the material.
    #[default]
    Outside,
    /// Inside the body: under the faces, in the material.
    Inside,
    /// Half of the thickness on each side of the faces.
    Both,
}

/// A solid of its own: faces of the body of an earlier feature given
/// `thickness`, to `side` of them. At its rim the layer is bounded by the
/// faces round them, continued. The body keeps its own body: this
/// feature makes a new one.
///
/// A face is referred to by its name. All faces are faces of one body:
/// the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thicken {
    pub faces: Vec<PersistentName>,
    pub thickness: Expr,
    #[serde(default)]
    pub side: ThickenSide,
}

impl Thicken {
    /// The feature whose body the faces are of: the one that made the
    /// first face referred to. `None` without faces.
    pub fn source(&self) -> Option<FeatureId> {
        self.faces.first().map(|first| first.feature)
    }
}

/// Faces of the body of an earlier feature taken out, and the body closed
/// again where they were by the faces round them: a hole, a boss or a
/// pocket filled from the faces it was cut into or stood on, a chamfer or
/// a fillet collapsed into the edge between its neighbours. The body goes
/// into this feature.
///
/// A face is referred to by its name. All faces are faces of one body:
/// the feature the first name starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteFace {
    pub faces: Vec<PersistentName>,
}

impl DeleteFace {
    /// The feature whose body the faces are of: the one that made the
    /// first face referred to. `None` without faces.
    pub fn source(&self) -> Option<FeatureId> {
        self.faces.first().map(|first| first.feature)
    }
}

/// The body of an earlier feature made hollow: walls of `thickness` are
/// left behind every face except those of `open`, which are taken away,
/// so the hollow is open there. Without open faces the hollow is a
/// cavity closed on all sides. The body goes into this feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shell {
    pub source: FeatureId,
    pub thickness: Expr,
    /// The faces left open, by name: faces of `source`.
    #[serde(default)]
    pub open: Vec<PersistentName>,
}

/// An ISO metric screw thread on a cylindrical face of the body of an
/// earlier feature: external, from the flat face `start` at one end of
/// the cylinder along it for `length`, with a run-out of a quarter turn.
/// The body goes into this feature. A thread is cosmetic unless
/// `modeled`: it changes no geometry, the face is only marked with what
/// the thread is; a modelled thread is cut into the body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The cylindrical face, by name.
    pub face: PersistentName,
    /// The flat face the thread starts at, square to the cylinder's axis,
    /// by name: a face of the same body.
    pub start: PersistentName,
    /// The size: a designation of ISO 261 (`M10`), a nominal diameter as
    /// an expression of a length (`d`), or `None` for the size whose
    /// nominal diameter the face has, found at every regeneration.
    #[serde(default)]
    pub size: Option<ThreadSize>,
    /// The pitch; `None` for the coarse pitch of ISO 261 of the size,
    /// looked up at every regeneration, so that it follows the size.
    #[serde(default)]
    pub pitch: Option<Expr>,
    pub length: Expr,
    #[serde(default)]
    pub left_handed: bool,
    /// The groove is cut into the body; otherwise the thread is cosmetic.
    #[serde(default)]
    pub modeled: bool,
}

impl Thread {
    /// The feature whose body the thread is cut in: the one that made the
    /// face.
    pub fn source(&self) -> FeatureId {
        self.face.feature
    }
}

/// The size of a thread, when it is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThreadSize {
    /// The designation of ISO 261: `M10`, `M2.5`. Its number is the
    /// nominal (major) diameter in millimetres, whatever the document's
    /// unit.
    Designation(String),
    /// The nominal (major) diameter: an expression of a length, `d`.
    Diameter { diameter: Expr },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FeatureKind {
    Block(Block),
    Cylinder(Cylinder),
    Sphere(Sphere),
    Cone(Cone),
    Torus(Torus),
    Sketch(Sketch),
    Extrude(Extrude),
    Revolve(Revolve),
    Loft(Loft),
    Sweep(Sweep),
    Transform(Transform),
    Mirror(Mirror),
    Pattern(Pattern),
    Boolean(Boolean),
    DatumPlane(DatumPlane),
    DatumAxis(DatumAxis),
    DatumPoint(DatumPoint),
    DatumHelix(DatumHelix),
    Chamfer(Chamfer),
    Fillet(Fillet),
    OffsetFace(OffsetFace),
    MoveFace(MoveFace),
    Thicken(Thicken),
    Draft(Draft),
    Shell(Shell),
    DeleteFace(DeleteFace),
    Thread(Thread),
    OffsetProfile(OffsetProfile),
    Fill(Fill),
    Split(Split),
    Import(Import),
}

impl FeatureKind {
    /// The kind's name, as in files and command results.
    pub fn type_name(&self) -> &'static str {
        match self {
            FeatureKind::Block(_) => "block",
            FeatureKind::Cylinder(_) => "cylinder",
            FeatureKind::Sphere(_) => "sphere",
            FeatureKind::Cone(_) => "cone",
            FeatureKind::Torus(_) => "torus",
            FeatureKind::Sketch(_) => "sketch",
            FeatureKind::Extrude(_) => "extrude",
            FeatureKind::Revolve(_) => "revolve",
            FeatureKind::Loft(_) => "loft",
            FeatureKind::Sweep(_) => "sweep",
            FeatureKind::Transform(_) => "transform",
            FeatureKind::Mirror(_) => "mirror",
            FeatureKind::Pattern(_) => "pattern",
            FeatureKind::Boolean(_) => "boolean",
            FeatureKind::DatumPlane(_) => "datumPlane",
            FeatureKind::DatumAxis(_) => "datumAxis",
            FeatureKind::DatumPoint(_) => "datumPoint",
            FeatureKind::DatumHelix(_) => "datumHelix",
            FeatureKind::Chamfer(_) => "chamfer",
            FeatureKind::Fillet(_) => "fillet",
            FeatureKind::OffsetFace(_) => "offsetFace",
            FeatureKind::MoveFace(_) => "moveFace",
            FeatureKind::Thicken(_) => "thicken",
            FeatureKind::Draft(_) => "draft",
            FeatureKind::Shell(_) => "shell",
            FeatureKind::DeleteFace(_) => "deleteFace",
            FeatureKind::Thread(_) => "thread",
            FeatureKind::OffsetProfile(_) => "offsetProfile",
            FeatureKind::Fill(_) => "fill",
            FeatureKind::Split(_) => "split",
            FeatureKind::Import(_) => "import",
        }
    }

    /// What a feature of this kind is called when it is given no name,
    /// before its number: `Block`, `Plane`.
    pub fn default_name(&self) -> String {
        match self {
            FeatureKind::DatumPlane(_) => "Plane".to_string(),
            FeatureKind::DatumAxis(_) => "Axis".to_string(),
            FeatureKind::DatumPoint(_) => "Point".to_string(),
            FeatureKind::DatumHelix(_) => "Helix".to_string(),
            other => {
                let mut base = other.type_name().to_string();
                if let Some(first) = base.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                base
            }
        }
    }

    /// True for reference geometry: a feature that makes no body and no
    /// sketch, only something to place other features by.
    pub fn is_datum(&self) -> bool {
        matches!(
            self,
            FeatureKind::DatumPlane(_)
                | FeatureKind::DatumAxis(_)
                | FeatureKind::DatumPoint(_)
                | FeatureKind::DatumHelix(_)
        )
    }

    /// The features whose bodies this feature is made from, each with
    /// whether it keeps its own body. Empty for a feature that makes its
    /// body from nothing or from a sketch. For a boolean: the target, then
    /// the tool.
    pub fn sources(&self) -> Vec<(FeatureId, bool)> {
        match self {
            FeatureKind::Boolean(b) => vec![(b.target, false), (b.tool, b.keep_tool)],
            other => other.source().into_iter().collect(),
        }
    }

    /// The one feature whose body this feature is made from (a transform,
    /// a mirror, a pattern, a chamfer, a fillet), and whether that feature keeps
    /// its own body.
    /// `None` for every other kind; see [`FeatureKind::sources`].
    pub fn source(&self) -> Option<(FeatureId, bool)> {
        match self {
            FeatureKind::Transform(t) => Some((t.source, t.keep)),
            FeatureKind::Mirror(m) => Some((m.source, m.keep)),
            FeatureKind::Pattern(p) => Some((p.source, false)),
            FeatureKind::Chamfer(c) => c.source().map(|id| (id, false)),
            FeatureKind::Fillet(f) => f.source().map(|id| (id, false)),
            FeatureKind::OffsetFace(o) => o.source().map(|id| (id, false)),
            FeatureKind::MoveFace(m) => m.source().map(|id| (id, false)),
            // The body stays: the layer is a body of its own.
            FeatureKind::Thicken(t) => t.source().map(|id| (id, true)),
            FeatureKind::Draft(d) => d.source().map(|id| (id, false)),
            FeatureKind::Shell(s) => Some((s.source, false)),
            FeatureKind::DeleteFace(d) => d.source().map(|id| (id, false)),
            FeatureKind::Thread(t) => Some((t.source(), false)),
            FeatureKind::Split(s) => Some((s.source, false)),
            FeatureKind::Block(_)
            | FeatureKind::Cylinder(_)
            | FeatureKind::Sphere(_)
            | FeatureKind::Cone(_)
            | FeatureKind::Torus(_)
            | FeatureKind::Sketch(_)
            | FeatureKind::Extrude(_)
            | FeatureKind::Revolve(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Sweep(_)
            | FeatureKind::Boolean(_)
            | FeatureKind::DatumPlane(_)
            | FeatureKind::DatumAxis(_)
            | FeatureKind::DatumPoint(_)
            | FeatureKind::DatumHelix(_)
            | FeatureKind::OffsetProfile(_)
            | FeatureKind::Fill(_)
            | FeatureKind::Import(_) => None,
        }
    }

    /// Every feature this one is made from, in a fixed order: the body it
    /// moves, the sketch it sweeps, the feature whose face it lies on, the
    /// datum it is placed by.
    pub fn inputs(&self) -> Vec<FeatureId> {
        match self {
            FeatureKind::Extrude(e) => match &e.end {
                ExtrudeEnd::UpToFace { face } => vec![e.sketch, face.feature],
                ExtrudeEnd::Distance | ExtrudeEnd::ThroughAll => vec![e.sketch],
            },
            FeatureKind::Revolve(r) => match r.axis {
                RevolveAxis::Datum { axis } => vec![r.sketch, axis],
                RevolveAxis::X | RevolveAxis::Y | RevolveAxis::Line { .. } => vec![r.sketch],
            },
            FeatureKind::Loft(l) => l.sections.iter().map(|s| s.sketch).collect(),
            FeatureKind::Sweep(s) => vec![s.sketch, s.path],
            FeatureKind::OffsetProfile(o) => vec![o.sketch],
            FeatureKind::Fill(f) => vec![f.sketch],
            FeatureKind::Sketch(sketch) => match &sketch.plane {
                SketchPlane::Face { face, .. } => vec![face.feature],
                SketchPlane::Datum { plane, .. } => vec![*plane],
                SketchPlane::World { .. } => Vec::new(),
            },
            FeatureKind::DatumPlane(datum) => datum.inputs(),
            FeatureKind::DatumAxis(datum) => datum.inputs(),
            FeatureKind::DatumPoint(datum) => datum.inputs(),
            FeatureKind::DatumHelix(datum) => datum.inputs(),
            FeatureKind::Pattern(p) => {
                let about = match &p.layout {
                    PatternLayout::Circular { about, .. } => *about,
                    PatternLayout::Linear { .. } => None,
                };
                std::iter::once(p.source).chain(about).collect()
            }
            FeatureKind::Mirror(m) => std::iter::once(m.source).chain(m.plane).collect(),
            FeatureKind::Draft(d) => d.source().into_iter().chain(d.neutral.inputs()).collect(),
            other => other.sources().into_iter().map(|(id, _)| id).collect(),
        }
    }

    /// True for a feature whose result depends on every body the features
    /// before it have made, whichever they are: an extrusion through all.
    pub fn uses_all_earlier(&self) -> bool {
        matches!(
            self,
            FeatureKind::Extrude(Extrude {
                end: ExtrudeEnd::ThroughAll,
                ..
            })
        )
    }

    /// Every expression of the feature with the name of its parameter, in a
    /// fixed order.
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        fn triple<'a>(name: &str, exprs: &'a [Expr; 3]) -> Vec<(String, &'a Expr)> {
            exprs
                .iter()
                .zip(["x", "y", "z"])
                .map(|(e, axis)| (format!("{name}.{axis}"), e))
                .collect()
        }
        fn turned(o: &Orientation) -> Vec<(String, &Expr)> {
            let mut out = triple("axis", &o.axis);
            out.push(("angle".to_string(), &o.angle));
            out
        }
        let named = |name: &str, e| (name.to_string(), e);
        let mut out;
        match self {
            FeatureKind::Block(b) => {
                out = triple("position", &b.position);
                out.extend(triple("size", &b.size));
                out.extend(turned(&b.orientation));
            }
            FeatureKind::Cylinder(c) => {
                out = triple("position", &c.position);
                out.extend([named("radius", &c.radius), named("height", &c.height)]);
                out.extend(turned(&c.orientation));
            }
            FeatureKind::Sphere(s) => {
                out = triple("position", &s.position);
                out.push(named("radius", &s.radius));
            }
            FeatureKind::Cone(c) => {
                out = triple("position", &c.position);
                out.extend([named("radius", &c.radius), named("height", &c.height)]);
                out.extend(turned(&c.orientation));
            }
            FeatureKind::Torus(t) => {
                out = triple("position", &t.position);
                out.extend([named("major", &t.major), named("minor", &t.minor)]);
                out.extend(turned(&t.orientation));
            }
            FeatureKind::Sketch(s) => out = s.exprs(),
            FeatureKind::Extrude(e) => out = vec![named("distance", &e.distance)],
            FeatureKind::OffsetProfile(o) => out = vec![named("distance", &o.distance)],
            FeatureKind::Revolve(r) => out = vec![named("angle", &r.angle)],
            FeatureKind::Transform(t) => {
                out = triple("translate", &t.translate);
                out.extend(triple("pivot", &t.pivot));
                out.extend(triple("axis", &t.axis));
                out.push(named("angle", &t.angle));
            }
            FeatureKind::Mirror(m) => {
                out = triple("point", &m.point);
                out.extend(triple("normal", &m.normal));
            }
            FeatureKind::Boolean(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Sweep(_)
            | FeatureKind::DeleteFace(_)
            | FeatureKind::Fill(_)
            | FeatureKind::Split(_)
            | FeatureKind::Import(_) => {
                out = Vec::new();
            }
            FeatureKind::DatumPlane(d) => out = d.exprs(),
            FeatureKind::DatumAxis(d) => out = d.exprs(),
            FeatureKind::DatumPoint(d) => out = d.exprs(),
            FeatureKind::DatumHelix(d) => out = d.exprs(),
            FeatureKind::Fillet(f) => out = vec![named("radius", &f.radius)],
            FeatureKind::OffsetFace(o) => out = vec![named("distance", &o.distance)],
            FeatureKind::Thicken(t) => out = vec![named("thickness", &t.thickness)],
            FeatureKind::Draft(d) => out = vec![named("angle", &d.angle)],
            FeatureKind::MoveFace(m) => {
                out = triple("translate", &m.translate);
                out.extend(triple("pivot", &m.pivot));
                out.extend(triple("axis", &m.axis));
                out.push(named("angle", &m.angle));
            }
            FeatureKind::Shell(s) => out = vec![named("thickness", &s.thickness)],
            FeatureKind::Thread(t) => {
                out = Vec::new();
                if let Some(ThreadSize::Diameter { diameter }) = &t.size {
                    out.push(named("size", diameter));
                }
                if let Some(pitch) = &t.pitch {
                    out.push(named("pitch", pitch));
                }
                out.push(named("length", &t.length));
            }
            FeatureKind::Chamfer(c) => {
                out = match &c.size {
                    ChamferSize::Equal { distance } => vec![named("size.distance", distance)],
                    ChamferSize::TwoDistances { distance, second } => {
                        vec![
                            named("size.distance", distance),
                            named("size.second", second),
                        ]
                    }
                    ChamferSize::DistanceAngle { distance, angle } => {
                        vec![named("size.distance", distance), named("size.angle", angle)]
                    }
                }
            }
            FeatureKind::Pattern(p) => {
                out = vec![named("count", &p.count)];
                match &p.layout {
                    PatternLayout::Linear { step } => out.extend(triple("step", step)),
                    PatternLayout::Circular {
                        point,
                        axis,
                        angle,
                        step,
                        ..
                    } => {
                        out.extend(triple("point", point));
                        out.extend(triple("axis", axis));
                        out.push(named("angle", angle));
                        out.extend(step.as_ref().map(|step| named("step", step)));
                    }
                }
            }
        }
        out
    }

    fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        fn turned(o: &mut Orientation) -> impl Iterator<Item = &mut Expr> {
            o.axis.iter_mut().chain([&mut o.angle])
        }
        match self {
            FeatureKind::Block(b) => b
                .position
                .iter_mut()
                .chain(b.size.iter_mut())
                .chain(turned(&mut b.orientation))
                .collect(),
            FeatureKind::Cylinder(c) => c
                .position
                .iter_mut()
                .chain([&mut c.radius, &mut c.height])
                .chain(turned(&mut c.orientation))
                .collect(),
            FeatureKind::Sphere(s) => s.position.iter_mut().chain([&mut s.radius]).collect(),
            FeatureKind::Cone(c) => c
                .position
                .iter_mut()
                .chain([&mut c.radius, &mut c.height])
                .chain(turned(&mut c.orientation))
                .collect(),
            FeatureKind::Torus(t) => t
                .position
                .iter_mut()
                .chain([&mut t.major, &mut t.minor])
                .chain(turned(&mut t.orientation))
                .collect(),
            FeatureKind::Sketch(s) => s.exprs_mut(),
            FeatureKind::Extrude(e) => vec![&mut e.distance],
            FeatureKind::OffsetProfile(o) => vec![&mut o.distance],
            FeatureKind::Revolve(r) => vec![&mut r.angle],
            FeatureKind::Transform(t) => t
                .translate
                .iter_mut()
                .chain(t.pivot.iter_mut())
                .chain(t.axis.iter_mut())
                .chain([&mut t.angle])
                .collect(),
            FeatureKind::Mirror(m) => m.point.iter_mut().chain(m.normal.iter_mut()).collect(),
            FeatureKind::Boolean(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Sweep(_)
            | FeatureKind::DeleteFace(_)
            | FeatureKind::Fill(_)
            | FeatureKind::Split(_)
            | FeatureKind::Import(_) => Vec::new(),
            FeatureKind::DatumPlane(d) => d.exprs_mut(),
            FeatureKind::DatumAxis(d) => d.exprs_mut(),
            FeatureKind::DatumPoint(d) => d.exprs_mut(),
            FeatureKind::DatumHelix(d) => d.exprs_mut(),
            FeatureKind::Fillet(f) => vec![&mut f.radius],
            FeatureKind::OffsetFace(o) => vec![&mut o.distance],
            FeatureKind::Thicken(t) => vec![&mut t.thickness],
            FeatureKind::Draft(d) => vec![&mut d.angle],
            FeatureKind::MoveFace(m) => m
                .translate
                .iter_mut()
                .chain(m.pivot.iter_mut())
                .chain(m.axis.iter_mut())
                .chain([&mut m.angle])
                .collect(),
            FeatureKind::Shell(s) => vec![&mut s.thickness],
            FeatureKind::Thread(t) => {
                let mut out = Vec::new();
                if let Some(ThreadSize::Diameter { diameter }) = &mut t.size {
                    out.push(diameter);
                }
                if let Some(pitch) = &mut t.pitch {
                    out.push(pitch);
                }
                out.push(&mut t.length);
                out
            }
            FeatureKind::Chamfer(c) => match &mut c.size {
                ChamferSize::Equal { distance } => vec![distance],
                ChamferSize::TwoDistances { distance, second } => vec![distance, second],
                ChamferSize::DistanceAngle { distance, angle } => vec![distance, angle],
            },
            FeatureKind::Pattern(p) => {
                let mut out = vec![&mut p.count];
                match &mut p.layout {
                    PatternLayout::Linear { step } => out.extend(step.iter_mut()),
                    PatternLayout::Circular {
                        point,
                        axis,
                        angle,
                        step,
                        ..
                    } => {
                        out.extend(point.iter_mut());
                        out.extend(axis.iter_mut());
                        out.push(angle);
                        out.extend(step.as_mut());
                    }
                }
                out
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    #[serde(default)]
    pub suppressed: bool,
    pub kind: FeatureKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DocError {
    Variable(VariableError),
    UnknownFeature(FeatureId),
    /// No feature has this name.
    UnknownFeatureName(String),
    InvalidFeatureName(&'static str),
    DuplicateFeatureName(String),
    TooManyFeatures,
    /// A reorder position past the end of the feature list.
    PositionOutOfRange {
        position: usize,
        len: usize,
    },
    /// The feature is not a sketch.
    NotASketch(FeatureId),
    /// A change to a sketch that cannot be made, with the sketch it is in.
    Sketch(FeatureId, SketchError),
    /// The document breaks one of its own rules (a file edited by hand, or
    /// written by something else).
    Invalid(String),
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocError::Variable(e) => write!(f, "{e}"),
            DocError::UnknownFeature(id) => write!(f, "there is no feature {id}"),
            DocError::UnknownFeatureName(name) => write!(f, "there is no feature named `{name}`"),
            DocError::InvalidFeatureName(rule) => write!(f, "invalid feature name: {rule}"),
            DocError::DuplicateFeatureName(name) => {
                write!(f, "a feature named `{name}` already exists")
            }
            DocError::TooManyFeatures => {
                write!(f, "a document holds at most {MAX_FEATURES} features")
            }
            DocError::PositionOutOfRange { position, len } => {
                write!(
                    f,
                    "position {position} is past the end of the {len} features"
                )
            }
            DocError::NotASketch(id) => write!(f, "feature {id} is not a sketch"),
            DocError::Sketch(id, e) => write!(f, "sketch {id}: {e}"),
            DocError::Invalid(what) => write!(f, "invalid document: {what}"),
        }
    }
}

impl std::error::Error for DocError {}

impl From<VariableError> for DocError {
    fn from(e: VariableError) -> Self {
        DocError::Variable(e)
    }
}

/// The whole design: what is saved, and what regeneration starts from.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Document {
    pub units: Units,
    pub tolerance: Tolerance,
    /// In the order they were defined; evaluation order follows dependencies.
    pub variables: Vec<Variable>,
    /// The design history, in order of application.
    pub features: Vec<Feature>,
    /// Regenerate up to and including this feature only.
    #[serde(default)]
    pub rollback: Option<FeatureId>,
    /// The id the next feature gets.
    pub next_feature_id: u64,
    /// The appearance and the material given to bodies, by the feature
    /// that makes each, in ascending order of feature id
    /// ([`appearance`]). Not part of the history: regeneration does not
    /// read it.
    #[serde(default)]
    pub bodies: Vec<BodyProperties>,
    /// The appearance given to single faces, by the feature whose body
    /// has the face and the face's persistent name there, in ascending
    /// order of the two ([`appearance`]). Not part of the history either.
    #[serde(default)]
    pub faces: Vec<FaceProperties>,
}

fn check_feature_name(name: &str) -> Result<(), DocError> {
    if name.is_empty() || name.trim() != name {
        return Err(DocError::InvalidFeatureName(
            "it must not be empty or start or end with a space",
        ));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(DocError::InvalidFeatureName("it is too long"));
    }
    if name.chars().any(char::is_control) {
        return Err(DocError::InvalidFeatureName(
            "it must not contain control characters",
        ));
    }
    Ok(())
}

/// [`check_feature_name`] for a name being given now (a feature added or
/// renamed): also not `F` followed by digits, the form of a feature's id,
/// so that `F3` always means the feature with id 3 and a reference by id
/// can never be taken for another feature by its name (2026-10-21). A
/// file that already has such a name still opens.
fn check_new_feature_name(name: &str) -> Result<(), DocError> {
    check_feature_name(name)?;
    if looks_like_id(name) {
        return Err(DocError::InvalidFeatureName(
            "it must not be F followed by digits, which is how a feature's id is written",
        ));
    }
    Ok(())
}

/// Whether `text` is written as a feature id: `F` and one or more digits.
pub fn looks_like_id(text: &str) -> bool {
    text.strip_prefix('F')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

impl Document {
    pub fn new(units: Units) -> Self {
        Self {
            units,
            next_feature_id: 1,
            ..Self::default()
        }
    }

    /// Checks the rules every document built through this API satisfies,
    /// for documents that come from elsewhere (a file): tolerances are
    /// positive and finite (the linear one within
    /// [`Tolerance::linear_in_range`]), variable and feature names are valid and
    /// unique, feature ids are unique and below `next_feature_id`, the
    /// rollback marker points at a feature, and the caps hold. Expressions
    /// are not evaluated: a formula that fails is a regeneration result,
    /// not an invalid document.
    pub fn validate(&self) -> Result<(), DocError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !(positive(self.tolerance.linear) && positive(self.tolerance.angular)) {
            return Err(DocError::Invalid(
                "tolerances must be positive and finite".to_string(),
            ));
        }
        if !Tolerance::linear_in_range(self.tolerance.linear) {
            return Err(DocError::Invalid(format!(
                "the linear tolerance {:e} is outside {:e} to {:e} of the length unit",
                self.tolerance.linear,
                Tolerance::LEAST_LINEAR,
                Tolerance::GREATEST_LINEAR
            )));
        }
        if let Some(loosest) = self.tolerance.loosest
            && !(loosest.is_finite() && loosest >= self.tolerance.linear)
        {
            return Err(DocError::Invalid(format!(
                "the cap on an entity's tolerance {loosest:e} is not a finite length at least the linear tolerance {:e}",
                self.tolerance.linear
            )));
        }
        self.variable_table()?;
        let mut names = std::collections::BTreeSet::new();
        if !self.variables.iter().all(|v| names.insert(v.name.as_str())) {
            return Err(DocError::Invalid(
                "two variables have the same name".to_string(),
            ));
        }
        if self.features.len() > MAX_FEATURES {
            return Err(DocError::TooManyFeatures);
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for feature in &self.features {
            check_feature_name(&feature.name)?;
            if !names.insert(feature.name.as_str()) {
                return Err(DocError::DuplicateFeatureName(feature.name.clone()));
            }
            if !ids.insert(feature.id) {
                return Err(DocError::Invalid(format!(
                    "two features have the id {}",
                    feature.id
                )));
            }
            if feature.id.0 == 0 || feature.id.0 >= self.next_feature_id {
                return Err(DocError::Invalid(format!(
                    "feature id {} is not below the next free id {}",
                    feature.id.0, self.next_feature_id
                )));
            }
            if let FeatureKind::Extrude(Extrude {
                end: ExtrudeEnd::UpToFace { face },
                ..
            }) = &feature.kind
            {
                face.check().map_err(|reason| {
                    DocError::Invalid(format!(
                        "the face `{}` ends on is not a name: {reason}",
                        feature.name
                    ))
                })?;
            }
            if let FeatureKind::Loft(loft) = &feature.kind
                && loft.sections.len() > MAX_LOFT_SECTIONS
            {
                return Err(DocError::Invalid(format!(
                    "the loft `{}` has {} sections; the most is {MAX_LOFT_SECTIONS}",
                    feature.name,
                    loft.sections.len()
                )));
            }
            let referred = match &feature.kind {
                FeatureKind::DatumPlane(datum) => datum.names(),
                FeatureKind::DatumAxis(datum) => datum.names(),
                FeatureKind::DatumPoint(datum) => datum.names(),
                FeatureKind::DatumHelix(datum) => datum.names(),
                FeatureKind::Chamfer(chamfer) => {
                    if chamfer.edges.len() > MAX_CHAMFER_EDGES {
                        return Err(DocError::Invalid(format!(
                            "`{}` has more than {MAX_CHAMFER_EDGES} edges",
                            feature.name
                        )));
                    }
                    chamfer.edges.iter().flatten().collect()
                }
                FeatureKind::Fillet(fillet) => {
                    if fillet.edges.len() > MAX_FILLET_EDGES {
                        return Err(DocError::Invalid(format!(
                            "`{}` has more than {MAX_FILLET_EDGES} edges",
                            feature.name
                        )));
                    }
                    fillet.edges.iter().flatten().collect()
                }
                FeatureKind::OffsetFace(OffsetFace { faces, .. })
                | FeatureKind::MoveFace(MoveFace { faces, .. })
                | FeatureKind::Thicken(Thicken { faces, .. })
                | FeatureKind::DeleteFace(DeleteFace { faces })
                | FeatureKind::Shell(Shell { open: faces, .. }) => {
                    if faces.len() > MAX_FACE_OP_FACES {
                        return Err(DocError::Invalid(format!(
                            "`{}` has more than {MAX_FACE_OP_FACES} faces",
                            feature.name
                        )));
                    }
                    faces.iter().collect()
                }
                FeatureKind::Thread(thread) => vec![&thread.face, &thread.start],
                FeatureKind::Split(split) => vec![&split.face],
                FeatureKind::Draft(draft) => {
                    if draft.faces.len() > MAX_FACE_OP_FACES {
                        return Err(DocError::Invalid(format!(
                            "`{}` has more than {MAX_FACE_OP_FACES} faces",
                            feature.name
                        )));
                    }
                    draft.faces.iter().chain(draft.neutral.names()).collect()
                }
                _ => Vec::new(),
            };
            for name in referred {
                name.check().map_err(|reason| {
                    DocError::Invalid(format!(
                        "a reference of `{}` is not a name: {reason}",
                        feature.name
                    ))
                })?;
            }
            if let FeatureKind::Sketch(sketch) = &feature.kind {
                sketch
                    .validate()
                    .map_err(|e| DocError::Sketch(feature.id, e))?;
                if let SketchPlane::Face { face, .. } = &sketch.plane {
                    face.check().map_err(|reason| {
                        DocError::Invalid(format!(
                            "the face `{}` lies on is not a name: {reason}",
                            feature.name
                        ))
                    })?;
                }
            }
        }
        self.validate_bodies()?;
        self.validate_faces()?;
        match self.rollback {
            Some(id) if !ids.contains(&id) => Err(DocError::UnknownFeature(id)),
            _ => Ok(()),
        }
    }

    /// The variables as an expression table, for evaluation.
    pub fn variable_table(&self) -> Result<VariableTable, VariableError> {
        let mut table = VariableTable::new();
        for v in &self.variables {
            table.set(&v.name, v.expr.source())?;
        }
        Ok(table)
    }

    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// Defines a variable or replaces its expression. The expression is not
    /// evaluated here: a formula that does not parse or refers to a missing
    /// variable is stored, and reported by regeneration.
    pub fn set_variable(&mut self, name: &str, expr: &str) -> Result<(), DocError> {
        // The table validates the name and the caps.
        let mut table = self.variable_table()?;
        table.set(name, expr)?;
        match self.variables.iter_mut().find(|v| v.name == name) {
            Some(v) => v.expr = Expr::new(expr),
            None => self.variables.push(Variable {
                name: name.to_string(),
                expr: Expr::new(expr),
            }),
        }
        Ok(())
    }

    /// Removes a variable. Expressions that still refer to it fail at the
    /// next regeneration with "unknown variable".
    pub fn remove_variable(&mut self, name: &str) -> Result<(), DocError> {
        let index = self
            .variables
            .iter()
            .position(|v| v.name == name)
            .ok_or_else(|| VariableError::UnknownVariable {
                name: name.to_string(),
            })?;
        self.variables.remove(index);
        Ok(())
    }

    /// Renames a variable everywhere: in the table, in other variables'
    /// expressions and in feature parameters. All or nothing.
    pub fn rename_variable(&mut self, old: &str, new: &str) -> Result<(), DocError> {
        let mut table = self.variable_table()?;
        table.rename(old, new)?;
        let mut renamed_features = Vec::with_capacity(self.features.len());
        for feature in &self.features {
            let mut kind = feature.kind.clone();
            for expr in kind.exprs_mut() {
                *expr = Expr::new(craft3d_expr::rename_variable(expr.source(), old, new)?);
            }
            renamed_features.push(kind);
        }
        self.variables = table
            .variables()
            .iter()
            .map(|v| Variable {
                name: v.name().to_string(),
                expr: Expr::new(v.source()),
            })
            .collect();
        for (feature, kind) in self.features.iter_mut().zip(renamed_features) {
            feature.kind = kind;
        }
        Ok(())
    }

    /// Renames every variable whose name has become reserved since the
    /// document was written (a function added to the expression language,
    /// such as `floor` in format version 17) to the first free name
    /// `{name}_1`, `{name}_2`, …, everywhere it is referred to, as
    /// [`Document::rename_variable`] does. Returns the renames made, old
    /// name first, in the order of the variables. For reading old files
    /// only: in such a file every token with that name is the variable.
    pub fn rename_reserved_variables(&mut self) -> Result<Vec<(String, String)>, DocError> {
        let mut renames: Vec<(String, String)> = Vec::new();
        let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for variable in &self.variables {
            used.insert(variable.name.clone());
            used.extend(
                craft3d_expr::names_in(variable.expr.source())
                    .into_iter()
                    .map(str::to_string),
            );
        }
        for feature in &mut self.features {
            for expr in feature.kind.exprs_mut() {
                used.extend(
                    craft3d_expr::names_in(expr.source())
                        .into_iter()
                        .map(str::to_string),
                );
            }
        }
        for index in 0..self.variables.len() {
            let Some(old) = self.variables.get(index).map(|v| v.name.clone()) else {
                break;
            };
            if !craft3d_expr::is_reserved_name(&old) {
                continue;
            }
            // A name in use is a variable's or any name an expression
            // mentions (a reference to a variable that is not there yet
            // would otherwise bind to the renamed one). Each candidate is
            // a distinct name, so the search ends within as many tries as
            // there are names in use, plus one.
            let taken = |name: &str| used.contains(name) || craft3d_expr::is_reserved_name(name);
            let Some(new) = (1..=used.len().saturating_add(1))
                .map(|k| format!("{old}_{k}"))
                .find(|name| !taken(name))
            else {
                return Err(DocError::Invalid(format!(
                    "the variable `{old}` has a name that is now reserved, and no free name was found for it"
                )));
            };
            let rename = |expr: &mut Expr| -> Result<(), DocError> {
                *expr = Expr::new(craft3d_expr::rename_reserved_variable(
                    expr.source(),
                    &old,
                    &new,
                )?);
                Ok(())
            };
            for variable in &mut self.variables {
                rename(&mut variable.expr)?;
            }
            for feature in &mut self.features {
                for expr in feature.kind.exprs_mut() {
                    rename(expr)?;
                }
            }
            if let Some(variable) = self.variables.get_mut(index) {
                variable.name.clone_from(&new);
            }
            used.insert(new.clone());
            renames.push((old, new));
        }
        Ok(renames)
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> Result<&mut Feature, DocError> {
        self.features
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or(DocError::UnknownFeature(id))
    }

    pub fn feature_by_name(&self, name: &str) -> Option<&Feature> {
        self.features.iter().find(|f| f.name == name)
    }

    /// The sketch a feature that takes closed profiles reads them from:
    /// a sketch, or an offset of the profiles of one (whose sketch is
    /// returned).
    pub fn profile_source(&self, id: FeatureId) -> Result<&Sketch, DocError> {
        match self.feature(id).map(|f| &f.kind) {
            Some(FeatureKind::OffsetProfile(offset)) => self.sketch(offset.sketch),
            _ => self.sketch(id),
        }
    }

    /// The sketch of a sketch feature.
    pub fn sketch(&self, id: FeatureId) -> Result<&Sketch, DocError> {
        match self.feature(id).map(|f| &f.kind) {
            Some(FeatureKind::Sketch(sketch)) => Ok(sketch),
            Some(_) => Err(DocError::NotASketch(id)),
            None => Err(DocError::UnknownFeature(id)),
        }
    }

    /// Changes a sketch. The change is all or nothing: when `change` fails
    /// the sketch is as it was.
    pub fn edit_sketch<T>(
        &mut self,
        id: FeatureId,
        change: impl FnOnce(&mut Sketch) -> Result<T, SketchError>,
    ) -> Result<T, DocError> {
        let mut sketch = self.sketch(id)?.clone();
        let out = change(&mut sketch).map_err(|e| DocError::Sketch(id, e))?;
        self.feature_mut(id)?.kind = FeatureKind::Sketch(sketch);
        Ok(out)
    }

    /// Appends a feature and returns its id. Without a name it is called
    /// after its kind and the first free number (`Block1`, `Block2`, …).
    pub fn add_feature(
        &mut self,
        name: Option<&str>,
        kind: FeatureKind,
    ) -> Result<FeatureId, DocError> {
        if self.features.len() >= MAX_FEATURES {
            return Err(DocError::TooManyFeatures);
        }
        let name = match name {
            Some(name) => {
                check_new_feature_name(name)?;
                if self.feature_by_name(name).is_some() {
                    return Err(DocError::DuplicateFeatureName(name.to_string()));
                }
                name.to_string()
            }
            None => {
                let base = kind.default_name();
                // At most len + 1 candidates can be taken.
                (1..=self.features.len() + 1)
                    .map(|n| format!("{base}{n}"))
                    .find(|candidate| self.feature_by_name(candidate).is_none())
                    .ok_or(DocError::TooManyFeatures)?
            }
        };
        let id = FeatureId(self.next_feature_id);
        self.next_feature_id = self
            .next_feature_id
            .checked_add(1)
            .ok_or(DocError::TooManyFeatures)?;
        self.features.push(Feature {
            id,
            name,
            suppressed: false,
            kind,
        });
        Ok(id)
    }

    pub fn rename_feature(&mut self, id: FeatureId, name: &str) -> Result<(), DocError> {
        check_new_feature_name(name)?;
        if self.features.iter().any(|f| f.name == name && f.id != id) {
            return Err(DocError::DuplicateFeatureName(name.to_string()));
        }
        self.feature_mut(id)?.name = name.to_string();
        Ok(())
    }

    /// Removes a feature. Its id is not reused.
    ///
    /// When the rollback marker is on it, the marker moves to the feature
    /// before it, so that what was rolled back stays rolled back. Deleting
    /// the first feature with the marker on it clears the marker: there is
    /// no feature left to put it on.
    pub fn remove_feature(&mut self, id: FeatureId) -> Result<Feature, DocError> {
        let index = self
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or(DocError::UnknownFeature(id))?;
        if self.rollback == Some(id) {
            self.rollback = index
                .checked_sub(1)
                .and_then(|before| self.features.get(before))
                .map(|f| f.id);
        }
        self.bodies.retain(|entry| entry.feature != id);
        self.faces.retain(|entry| entry.feature != id);
        Ok(self.features.remove(index))
    }

    /// Moves a feature to `position` in the history (0 is first).
    pub fn move_feature(&mut self, id: FeatureId, position: usize) -> Result<(), DocError> {
        let index = self
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or(DocError::UnknownFeature(id))?;
        if position >= self.features.len() {
            return Err(DocError::PositionOutOfRange {
                position,
                len: self.features.len(),
            });
        }
        let feature = self.features.remove(index);
        self.features.insert(position, feature);
        Ok(())
    }

    /// Sets the rollback marker; `None` regenerates everything.
    pub fn set_rollback(&mut self, id: Option<FeatureId>) -> Result<(), DocError> {
        if let Some(id) = id
            && self.feature(id).is_none()
        {
            return Err(DocError::UnknownFeature(id));
        }
        self.rollback = id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exprs(values: [&str; 3]) -> [Expr; 3] {
        values.map(Expr::new)
    }

    fn block(size: [&str; 3]) -> FeatureKind {
        FeatureKind::Block(Block {
            position: exprs(["0 mm", "0 mm", "0 mm"]),
            size: exprs(size),
            orientation: Orientation::default(),
        })
    }

    fn sample() -> Document {
        let mut doc = Document::new(Units::default());
        doc.set_variable("width", "40 mm").unwrap();
        doc.set_variable("half", "width / 2").unwrap();
        doc.add_feature(None, block(["width", "half", "10 mm"]))
            .unwrap();
        doc.add_feature(
            Some("Pin"),
            FeatureKind::Cylinder(Cylinder {
                position: exprs(["half", "0 mm", "10 mm"]),
                radius: Expr::new("width / 8"),
                height: Expr::new("2 * half"),
                orientation: Orientation::default(),
            }),
        )
        .unwrap();
        doc
    }

    #[test]
    fn a_screw_pattern_lists_its_step_after_its_angle_and_follows_a_rename() {
        let mut doc = Document::new(Units::default());
        doc.variables.push(Variable {
            name: "rise".to_string(),
            expr: Expr::new("100 mm"),
        });
        doc.add_feature(None, block(["10", "10", "10"])).unwrap();
        let circular = |step: Option<&str>| {
            FeatureKind::Pattern(Pattern {
                source: FeatureId(1),
                count: Expr::new("4"),
                layout: PatternLayout::Circular {
                    point: exprs(["0", "0", "0"]),
                    axis: exprs(["0", "0", "1"]),
                    angle: Expr::new("90 deg"),
                    about: None,
                    step: step.map(Expr::new),
                },
            })
        };
        doc.add_feature(None, circular(Some("rise / 4"))).unwrap();
        doc.add_feature(None, circular(None)).unwrap();
        let names = |kind: &FeatureKind| -> Vec<String> {
            kind.exprs().into_iter().map(|(name, _)| name).collect()
        };
        // A plain turn has no `step`; a screw lists it last.
        assert_eq!(
            names(&doc.features[2].kind),
            [
                "count", "point.x", "point.y", "point.z", "axis.x", "axis.y", "axis.z", "angle"
            ]
        );
        assert_eq!(
            names(&doc.features[1].kind),
            [
                "count", "point.x", "point.y", "point.z", "axis.x", "axis.y", "axis.z", "angle",
                "step"
            ]
        );
        doc.rename_variable("rise", "storey").unwrap();
        let (_, step) = doc.features[1].kind.exprs().pop().unwrap();
        assert_eq!(step.source(), "storey / 4");
    }

    #[test]
    fn variables_whose_names_became_functions_are_renamed() {
        let mut doc = Document::new(Units::default());
        // As an old file has them: `floor` and `mod` were free names.
        for (name, expr) in [
            ("floor", "3 mm"),
            ("floor_1", "1 mm"),
            ("mod", "floor * 2"),
            ("total", "floor + mod + floor_1"),
        ] {
            doc.variables.push(Variable {
                name: name.to_string(),
                expr: Expr::new(expr),
            });
        }
        doc.add_feature(None, block(["floor", "mod*floor", "floor_1"]))
            .unwrap();
        assert!(doc.validate().is_err());
        let renames = doc.rename_reserved_variables().unwrap();
        assert_eq!(
            renames,
            [
                ("floor".to_string(), "floor_2".to_string()),
                ("mod".to_string(), "mod_1".to_string())
            ]
        );
        let sources: Vec<(&str, &str)> = doc
            .variables
            .iter()
            .map(|v| (v.name.as_str(), v.expr.source()))
            .collect();
        assert_eq!(
            sources,
            [
                ("floor_2", "3 mm"),
                ("floor_1", "1 mm"),
                ("mod_1", "floor_2 * 2"),
                ("total", "floor_2 + mod_1 + floor_1"),
            ]
        );
        let FeatureKind::Block(b) = &doc.features[0].kind else {
            panic!("not a block")
        };
        assert_eq!(b.size[1].source(), "mod_1*floor_2");
        doc.validate().unwrap();
        // Nothing to do the second time.
        assert!(doc.rename_reserved_variables().unwrap().is_empty());
    }

    /// A name an expression mentions without a variable behind it (it
    /// fails as "unknown variable") is not free: renaming `floor` to it
    /// would make that reference bind silently.
    #[test]
    fn a_renamed_variable_does_not_take_a_name_an_expression_mentions() {
        let mut doc = Document::new(Units::default());
        doc.variables.push(Variable {
            name: "floor".to_string(),
            expr: Expr::new("3 mm"),
        });
        doc.variables.push(Variable {
            name: "a".to_string(),
            expr: Expr::new("floor_1 + 1 mm"),
        });
        doc.add_feature(None, block(["floor", "floor_2", "1 mm"]))
            .unwrap();
        let renames = doc.rename_reserved_variables().unwrap();
        assert_eq!(renames, [("floor".to_string(), "floor_3".to_string())]);
        assert_eq!(doc.variables[1].expr.source(), "floor_1 + 1 mm");
        let FeatureKind::Block(b) = &doc.features[0].kind else {
            panic!("not a block")
        };
        assert_eq!(b.size[0].source(), "floor_3");
        assert_eq!(b.size[1].source(), "floor_2");
    }

    #[test]
    fn features_get_stable_ids_and_readable_names() {
        let mut doc = sample();
        assert_eq!(doc.features[0].id, FeatureId(1));
        assert_eq!(doc.features[0].name, "Block1");
        assert_eq!(doc.features[1].name, "Pin");
        let third = doc
            .add_feature(None, block(["1 mm", "1 mm", "1 mm"]))
            .unwrap();
        assert_eq!(
            (third, doc.feature(third).unwrap().name.as_str()),
            (FeatureId(3), "Block2")
        );
        // Ids are never reused, names are.
        doc.remove_feature(third).unwrap();
        let fourth = doc
            .add_feature(None, block(["1 mm", "1 mm", "1 mm"]))
            .unwrap();
        assert_eq!(
            (fourth, doc.feature(fourth).unwrap().name.as_str()),
            (FeatureId(4), "Block2")
        );
        assert_eq!(FeatureId(4).to_string(), "F4");
        assert!(matches!(
            doc.remove_feature(third),
            Err(DocError::UnknownFeature(FeatureId(3)))
        ));
    }

    #[test]
    fn feature_names_are_validated_and_unique() {
        let mut doc = sample();
        let kind = || block(["1 mm", "1 mm", "1 mm"]);
        assert!(matches!(
            doc.add_feature(Some("Pin"), kind()),
            Err(DocError::DuplicateFeatureName(_))
        ));
        for bad in ["", " x", "x ", "a\nb"] {
            assert!(
                matches!(
                    doc.add_feature(Some(bad), kind()),
                    Err(DocError::InvalidFeatureName(_))
                ),
                "`{bad}`"
            );
        }
        assert!(
            doc.add_feature(Some(&"n".repeat(MAX_NAME_LEN + 1)), kind())
                .is_err()
        );
        assert!(doc.rename_feature(FeatureId(1), "Pin").is_err());
        doc.rename_feature(FeatureId(1), "Base plate").unwrap();
        doc.rename_feature(FeatureId(1), "Base plate").unwrap();
        assert_eq!(doc.feature_by_name("Base plate").unwrap().id, FeatureId(1));
        assert_eq!(doc.features.len(), 2, "failed adds add nothing");
        assert_eq!(doc.next_feature_id, 3, "and use up no ids");
    }

    #[test]
    fn renaming_a_variable_follows_it_into_features() {
        let mut doc = sample();
        doc.rename_variable("half", "h").unwrap();
        assert_eq!(doc.variable("h").unwrap().expr.source(), "width / 2");
        let FeatureKind::Block(b) = &doc.features[0].kind else {
            panic!()
        };
        assert_eq!(b.size[1].source(), "h");
        let FeatureKind::Cylinder(c) = &doc.features[1].kind else {
            panic!()
        };
        assert_eq!((c.position[0].source(), c.height.source()), ("h", "2 * h"));
        // A failed rename changes nothing.
        let before = doc.clone();
        assert!(doc.rename_variable("h", "width").is_err());
        assert!(doc.rename_variable("h", "mm").is_err());
        assert!(doc.rename_variable("missing", "x").is_err());
        assert_eq!(doc, before);
    }

    fn sketched() -> (Document, FeatureId) {
        let mut doc = sample();
        let id = doc
            .add_feature(None, FeatureKind::Sketch(Sketch::default()))
            .unwrap();
        doc.edit_sketch(id, |s| {
            let a = s.add_point(0.0, 0.0)?;
            let b = s.add_point(30.0, 0.0)?;
            let line = s.add_line(a, b, false)?;
            s.add_constraint(ConstraintKind::Distance {
                a: line,
                b: None,
                direction: DistanceDirection::Direct,
                value: Expr::new("half + 10 mm"),
            })
        })
        .unwrap();
        (doc, id)
    }

    #[test]
    fn a_sketch_is_a_feature_whose_dimensions_follow_variables() {
        let (mut doc, id) = sketched();
        assert_eq!(doc.feature(id).unwrap().name, "Sketch1");
        assert_eq!(doc.validate(), Ok(()));
        doc.rename_variable("half", "h").unwrap();
        let sketch = doc.sketch(id).unwrap();
        assert_eq!(
            sketch.constraints[0].kind.value().unwrap().source(),
            "h + 10 mm"
        );
        let names: Vec<String> = doc
            .feature(id)
            .unwrap()
            .kind
            .exprs()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["plane.offset", "c1.value"]);
        // Not a sketch, and not there.
        assert_eq!(
            doc.sketch(FeatureId(1)).unwrap_err(),
            DocError::NotASketch(FeatureId(1))
        );
        assert_eq!(
            doc.sketch(FeatureId(99)).unwrap_err(),
            DocError::UnknownFeature(FeatureId(99))
        );
    }

    #[test]
    fn an_extrusion_points_at_its_sketch_and_follows_variables() {
        let (mut doc, sketch) = sketched();
        let id = doc
            .add_feature(
                None,
                FeatureKind::Extrude(Extrude {
                    sketch,
                    distance: Expr::new("2 * half"),
                    symmetric: false,
                    end: Default::default(),
                    reversed: false,
                }),
            )
            .unwrap();
        assert_eq!(doc.feature(id).unwrap().name, "Extrude1");
        doc.rename_variable("half", "h").unwrap();
        let kind = &doc.feature(id).unwrap().kind;
        assert_eq!(
            kind.exprs()
                .iter()
                .map(|(n, e)| (n.as_str(), e.source()))
                .collect::<Vec<_>>(),
            [("distance", "2 * h")]
        );
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::json!({"type": "extrude", "sketch": 3, "distance": "2 * h", "symmetric": false,
                "end": {"type": "distance"}, "reversed": false})
        );
        assert_eq!(kind.inputs(), [sketch]);
        assert!(!kind.uses_all_earlier());
        // `symmetric` may be absent. A sketch that is gone is not an
        // invalid document: regeneration reports it.
        let read: FeatureKind = serde_json::from_value(
            serde_json::json!({"type": "extrude", "sketch": 3, "distance": "1"}),
        )
        .unwrap();
        assert!(matches!(
            read,
            FeatureKind::Extrude(Extrude {
                symmetric: false,
                ..
            })
        ));
        doc.remove_feature(sketch).unwrap();
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn the_end_of_an_extrusion_says_what_it_depends_on() {
        let (mut doc, sketch) = sketched();
        let face = PersistentName::new(FeatureId(7), NameRole::Face, vec![1]);
        let extrusion = |end| {
            FeatureKind::Extrude(Extrude {
                sketch,
                distance: Expr::new("0"),
                symmetric: false,
                end,
                reversed: true,
            })
        };
        let up_to = extrusion(ExtrudeEnd::UpToFace { face: face.clone() });
        assert_eq!(up_to.inputs(), [sketch, FeatureId(7)]);
        assert!(!up_to.uses_all_earlier());
        assert_eq!(
            serde_json::to_value(&up_to).unwrap()["end"],
            serde_json::json!({"type": "upToFace",
                "face": serde_json::to_value(&face).unwrap()})
        );
        let through = extrusion(ExtrudeEnd::ThroughAll);
        assert_eq!(through.inputs(), [sketch]);
        assert!(through.uses_all_earlier());
        assert_eq!(ExtrudeEnd::default().type_name(), "distance");
        // A name that could not be written back is an invalid document.
        let id = doc.add_feature(None, up_to).unwrap();
        assert_eq!(doc.validate(), Ok(()));
        let broken = PersistentName {
            of: None,
            ..PersistentName::copy(FeatureId(7), 0, face)
        };
        doc.feature_mut(id).unwrap().kind = extrusion(ExtrudeEnd::UpToFace { face: broken });
        let error = doc.validate().unwrap_err().to_string();
        assert!(error.contains("ends on is not a name"), "{error}");
    }

    #[test]
    fn a_datum_plane_says_what_it_is_placed_by_and_follows_variables() {
        let mut doc = sample();
        let face = |n| PersistentName::new(FeatureId(1), NameRole::Face, vec![n]);
        let offset = FeatureKind::DatumPlane(DatumPlane {
            by: PlaneBy::Offset {
                from: PlaneRef::Face { face: face(1) },
                distance: Expr::new("half + 1 mm"),
            },
        });
        assert_eq!(offset.inputs(), [FeatureId(1)]);
        assert!(offset.is_datum() && offset.source().is_none());
        assert_eq!(
            serde_json::to_value(&offset).unwrap(),
            serde_json::json!({"type": "datumPlane", "by": {"type": "offset",
                "from": {"type": "face", "face": "F1:face(1)"}, "distance": "half + 1 mm"}})
        );
        let shelf = doc.add_feature(None, offset).unwrap();
        assert_eq!(doc.feature(shelf).unwrap().name, "Plane1");
        let angled = FeatureKind::DatumPlane(DatumPlane {
            by: PlaneBy::Angle {
                from: PlaneRef::Datum { feature: shelf },
                about: AxisRef::Edge {
                    between: [
                        face(1),
                        PersistentName::new(FeatureId(2), NameRole::Face, vec![0]),
                    ],
                },
                angle: Expr::new("half * 1 deg"),
            },
        });
        assert_eq!(angled.inputs(), [shelf, FeatureId(1), FeatureId(2)]);
        let through = FeatureKind::DatumPlane(DatumPlane {
            by: PlaneBy::ThreePoints {
                points: [
                    PointRef::At {
                        position: exprs(["half", "0", "0"]),
                    },
                    PointRef::Vertex {
                        vertex: PersistentName::new(FeatureId(2), NameRole::Vertex, vec![3]),
                    },
                    PointRef::At {
                        position: exprs(["0", "width", "0"]),
                    },
                ],
            },
        });
        assert_eq!(through.inputs(), [FeatureId(2)]);
        assert_eq!(
            through
                .exprs()
                .iter()
                .map(|(n, e)| (n.as_str(), e.source()))
                .collect::<Vec<_>>(),
            [
                ("through.0.x", "half"),
                ("through.0.y", "0"),
                ("through.0.z", "0"),
                ("through.2.x", "0"),
                ("through.2.y", "width"),
                ("through.2.z", "0")
            ]
        );
        let angled = doc.add_feature(None, angled).unwrap();
        let through = doc.add_feature(None, through).unwrap();
        // What lies on a datum, or is mirrored in one, depends on it.
        let sketch = FeatureKind::Sketch(Sketch::new(SketchPlane::Datum {
            plane: shelf,
            offset: Expr::new("half"),
        }));
        assert_eq!(sketch.inputs(), [shelf]);
        let mirror = FeatureKind::Mirror(Mirror {
            source: FeatureId(1),
            point: exprs(["0", "0", "0"]),
            normal: exprs(["0", "0", "1"]),
            plane: Some(angled),
            keep: false,
        });
        assert_eq!(mirror.inputs(), [FeatureId(1), angled]);
        assert_eq!(mirror.sources(), [(FeatureId(1), false)]);
        // A mirror written before datums existed has none.
        let read: FeatureKind = serde_json::from_value(serde_json::json!(
            {"type": "mirror", "source": 1, "normal": ["0", "0", "1"]}
        ))
        .unwrap();
        assert!(matches!(
            read,
            FeatureKind::Mirror(Mirror { plane: None, .. })
        ));
        doc.add_feature(None, sketch).unwrap();
        assert_eq!(doc.validate(), Ok(()));
        // Variables are renamed in every expression of a datum.
        doc.rename_variable("half", "h").unwrap();
        let sources = |id| -> Vec<String> {
            doc.feature(id)
                .unwrap()
                .kind
                .exprs()
                .iter()
                .map(|(_, e)| e.source().to_string())
                .collect()
        };
        assert_eq!(sources(shelf), ["h + 1 mm"]);
        assert_eq!(sources(angled), ["h * 1 deg"]);
        assert_eq!(sources(through)[0], "h");
        // A name that could not be written back is an invalid document.
        let broken = PersistentName {
            of: None,
            ..PersistentName::copy(FeatureId(7), 0, face(1))
        };
        doc.feature_mut(shelf).unwrap().kind = FeatureKind::DatumPlane(DatumPlane {
            by: PlaneBy::Offset {
                from: PlaneRef::Face { face: broken },
                distance: Expr::new("1"),
            },
        });
        let error = doc.validate().unwrap_err().to_string();
        assert!(
            error.contains("a reference of `Plane1` is not a name"),
            "{error}"
        );
    }

    #[test]
    fn a_failed_sketch_edit_changes_nothing() {
        let (mut doc, id) = sketched();
        let before = doc.clone();
        let error = doc
            .edit_sketch(id, |s| {
                s.add_point(5.0, 5.0)?;
                s.add_line(EntityId(1), EntityId(77), false)
            })
            .unwrap_err();
        assert_eq!(
            error,
            DocError::Sketch(id, SketchError::UnknownEntity(EntityId(77)))
        );
        assert_eq!(error.to_string(), "sketch F3: the sketch has no entity e77");
        assert_eq!(doc, before);
        // A sketch damaged behind the API makes the document invalid.
        let FeatureKind::Sketch(sketch) = &mut doc.features[2].kind else {
            panic!()
        };
        sketch.next_entity_id = 1;
        assert!(matches!(doc.validate(), Err(DocError::Sketch(..))));
    }

    #[test]
    fn variables_can_be_set_replaced_and_removed() {
        let mut doc = sample();
        doc.set_variable("width", "2 in").unwrap();
        assert_eq!(doc.variables.len(), 2);
        assert_eq!(doc.variables[0].expr.source(), "2 in");
        // Stored even when it does not evaluate: regeneration reports it.
        doc.set_variable("broken", "width +").unwrap();
        assert!(doc.set_variable("sin", "1").is_err());
        assert!(doc.set_variable("1x", "1").is_err());
        doc.remove_variable("half").unwrap();
        assert!(doc.remove_variable("half").is_err());
        assert_eq!(
            doc.variables
                .iter()
                .map(|v| v.name.as_str())
                .collect::<Vec<_>>(),
            ["width", "broken"]
        );
    }

    #[test]
    fn features_can_be_reordered_and_rolled_back() {
        let mut doc = sample();
        doc.move_feature(FeatureId(2), 0).unwrap();
        assert_eq!(
            doc.features.iter().map(|f| f.id.0).collect::<Vec<_>>(),
            [2, 1]
        );
        assert!(matches!(
            doc.move_feature(FeatureId(2), 2),
            Err(DocError::PositionOutOfRange { .. })
        ));
        assert!(doc.move_feature(FeatureId(9), 0).is_err());
        doc.set_rollback(Some(FeatureId(2))).unwrap();
        assert!(doc.set_rollback(Some(FeatureId(9))).is_err());
        assert_eq!(doc.rollback, Some(FeatureId(2)));
        // Deleting the marked feature when it is the first clears the
        // marker.
        doc.remove_feature(FeatureId(2)).unwrap();
        assert_eq!(doc.rollback, None);
    }

    #[test]
    fn deleting_the_marked_feature_leaves_the_marker_on_the_one_before() {
        let mut doc = sample();
        let ids: Vec<FeatureId> = doc.features.iter().map(|f| f.id).collect();
        assert!(ids.len() >= 2, "the sample has several features");
        let (before, marked) = (ids[ids.len() - 2], ids[ids.len() - 1]);
        doc.set_rollback(Some(marked)).unwrap();
        doc.remove_feature(marked).unwrap();
        assert_eq!(doc.rollback, Some(before));
        assert!(doc.validate().is_ok());
        // Deleting another feature leaves the marker where it is.
        let mut doc = sample();
        doc.set_rollback(Some(marked)).unwrap();
        doc.remove_feature(before).unwrap();
        assert_eq!(doc.rollback, Some(marked));
    }

    #[test]
    fn every_expression_of_a_feature_is_listed_with_its_parameter() {
        let doc = sample();
        let names: Vec<String> = doc.features[1]
            .kind
            .exprs()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            names,
            [
                "position.x",
                "position.y",
                "position.z",
                "radius",
                "height",
                "axis.x",
                "axis.y",
                "axis.z",
                "angle"
            ]
        );
        assert_eq!(doc.features[0].kind.exprs().len(), 10);
    }

    #[test]
    fn validation_accepts_built_documents_and_rejects_broken_ones() {
        let doc = sample();
        assert_eq!(doc.validate(), Ok(()));
        assert_eq!(Document::new(Units::default()).validate(), Ok(()));
        type Damage = fn(&mut Document);
        let broken: [(&str, Damage); 13] = [
            ("zero tolerance", |d| d.tolerance.linear = 0.0),
            ("vanishing tolerance", |d| d.tolerance.linear = 1e-300),
            ("tolerance above the range", |d| d.tolerance.linear = 2e3),
            ("cap below the tolerance", |d| {
                d.tolerance.loosest = Some(1e-9)
            }),
            ("cap not finite", |d| d.tolerance.loosest = Some(f64::NAN)),
            ("NaN tolerance", |d| d.tolerance.angular = f64::NAN),
            ("reserved variable name", |d| {
                d.variables[0].name = "mm".into()
            }),
            ("duplicate variable", |d| {
                d.variables[1].name = "width".into()
            }),
            ("duplicate feature id", |d| d.features[1].id = FeatureId(1)),
            ("id not below next", |d| d.next_feature_id = 2),
            ("id zero", |d| d.features[0].id = FeatureId(0)),
            ("duplicate feature name", |d| {
                d.features[1].name = "Block1".into()
            }),
            ("dangling rollback", |d| d.rollback = Some(FeatureId(7))),
        ];
        for (what, damage) in broken {
            let mut doc = sample();
            damage(&mut doc);
            assert!(doc.validate().is_err(), "{what}");
        }
        let mut bad_name = sample();
        bad_name.features[0].name = String::new();
        assert!(bad_name.validate().is_err());
    }

    #[test]
    fn the_json_form_is_stable_and_round_trips() {
        let doc = sample();
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            json["units"],
            serde_json::json!({"length": "mm", "angle": "deg"})
        );
        assert_eq!(
            json["variables"][1],
            serde_json::json!({"name": "half", "expr": "width / 2"})
        );
        assert_eq!(
            json["features"][0],
            serde_json::json!({
                "id": 1, "name": "Block1", "suppressed": false,
                "kind": {"type": "block", "position": ["0 mm", "0 mm", "0 mm"], "size": ["width", "half", "10 mm"],
                         "orientation": {"axis": ["0", "0", "1"], "angle": "0 deg"}}
            })
        );
        let back: Document = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, doc);
        // A feature written before orientations existed still reads, upright.
        let mut older = json;
        older["features"][0]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("orientation");
        assert_eq!(serde_json::from_value::<Document>(older).unwrap(), doc);
        for unit in LengthUnit::ALL {
            assert_eq!(LengthUnit::from_symbol(unit.symbol()), Some(unit));
            assert_eq!(
                serde_json::to_value(unit).unwrap(),
                serde_json::json!(unit.symbol())
            );
        }
    }
}
