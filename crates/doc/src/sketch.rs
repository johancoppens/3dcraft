//! Sketches: 2D entities on a plane, tied together by constraints.
//!
//! Pure data, like the rest of this crate. A sketch stores what the user
//! drew and constrained, in the order it was done, plus the coordinates of
//! the last solution: the numbers in [`Geometry::Point`] and
//! [`Geometry::Circle`] are where the solver left the sketch. They are the
//! solver's starting point next time, so a reopened sketch does not flip to
//! a mirror solution (docs/design/document.md). Dimension values are
//! expressions; coordinates are plain numbers in the document's length unit.
//!
//! Entities and constraints are addressed by ids that are never reused
//! within their sketch: persistent names will be built on them.

use crate::Expr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Largest number of entities in one sketch.
pub const MAX_SKETCH_ENTITIES: usize = 10_000;
/// Largest number of constraints in one sketch.
pub const MAX_SKETCH_CONSTRAINTS: usize = 20_000;
/// Largest number of patterns in one sketch.
pub const MAX_SKETCH_PATTERNS: usize = 1_000;
/// Most instances of one sketch pattern, the seed included and both
/// directions of a grid together.
pub const MAX_PATTERN_INSTANCES: usize = 10_000;
/// Most levels of a tree of sketch patterns: a pattern that holds a
/// pattern that holds a pattern. Each level is one bracket of a name
/// (`e3[2][1][4]`, [`crate::CurvePath`]).
pub const MAX_PATTERN_DEPTH: usize = crate::name::MAX_CURVE_LEVELS;
/// Highest entity id a sketch gives out: a persistent name keeps the id of
/// a curve in 32 bits and the instance of a pattern above it
/// ([`crate::CurveRef`]).
pub const MAX_ENTITY_ID: u64 = crate::name::MAX_CURVE_ENTITY;
/// Highest degree of a spline. A sketch curve gains nothing from more, and
/// everything downstream (areas, intersections, the surfaces swept from
/// it) costs with the degree.
pub const MAX_SPLINE_DEGREE: u32 = 7;
/// Largest number of control points of one spline.
pub const MAX_SPLINE_POINTS: usize = 1_000;

/// Identifies an entity within its sketch. Written `e3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub u64);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// Identifies a constraint within its sketch. Written `c3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConstraintId(pub u64);

impl fmt::Display for ConstraintId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// Identifies a pattern within its sketch. Written `p3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatternId(pub u64);

impl fmt::Display for PatternId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// Which instance of a sketch pattern an image point is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceIndex {
    /// Instance `k` of a pattern in one direction (0 is the seed). Past
    /// the count it is missing: it never stands for another instance.
    At(u32),
    /// Instance `i, j` of a grid.
    Grid([u32; 2]),
    /// The last instance, whatever the count: a declared relative address.
    Last,
}

impl fmt::Display for InstanceIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceIndex::At(k) => write!(f, "{k}"),
            InstanceIndex::Grid([i, j]) => write!(f, "{i},{j}"),
            InstanceIndex::Last => f.write_str("last"),
        }
    }
}

impl InstanceIndex {
    /// Reads the part between the brackets of `e5[3]`, `e5[2,4]` or
    /// `e5[last]`.
    pub fn parse(text: &str) -> Option<Self> {
        let number = |t: &str| -> Option<u32> {
            let t = t.trim();
            (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
                .then(|| t.parse::<u32>().ok())
                .flatten()
        };
        match text.split(',').collect::<Vec<_>>().as_slice() {
            [one] if one.trim() == "last" => Some(InstanceIndex::Last),
            [k] => Some(InstanceIndex::At(number(k)?)),
            [i, j] => Some(InstanceIndex::Grid([number(i)?, number(j)?])),
            _ => None,
        }
    }
}

/// An image point as text: `e5[3]`, `e5[2,4]`, `e5[last]`. `None` for a
/// text without brackets; an error for one with brackets that do not read.
pub fn parse_image_ref(text: &str) -> Option<Result<(EntityId, InstanceIndex), String>> {
    let (entity, rest) = text.trim().split_once('[')?;
    let shown: String = text.chars().take(40).collect();
    let wrong = || {
        format!(
            "`{shown}` is not an image point: write the seed point and the instance, as e5[3], e5[2,4] in a grid, or e5[last]"
        )
    };
    let Some(inner) = rest.strip_suffix(']') else {
        return Some(Err(wrong()));
    };
    let entity = entity.trim();
    let id = entity
        .strip_prefix('e')
        .unwrap_or(entity)
        .parse::<u64>()
        .ok()
        .filter(|_| !entity.is_empty());
    match (id, InstanceIndex::parse(inner)) {
        (Some(id), Some(index)) => Some(Ok((EntityId(id), index))),
        _ => Some(Err(wrong())),
    }
}

/// An image point as text with an instance for each level of a tree of
/// patterns, the innermost first: `e8[2]`, `e8[last][last]` (the last
/// instance of the inner pattern in the last of the outer one),
/// `e8[1][2,4]`. `None` for a text without brackets; an error for one
/// with brackets that do not read, or with more levels than a tree has.
pub fn parse_image_path(text: &str) -> Option<Result<(EntityId, Vec<InstanceIndex>), String>> {
    let (entity, mut rest) = text.trim().split_once('[')?;
    let shown: String = text.chars().take(40).collect();
    let wrong = || {
        format!(
            "`{shown}` is not an image point: write the seed point and the instance, as e5[3], e5[2,4] in a grid, or e5[last], and for nested patterns the instance of each pattern around it after it, the innermost first, as e5[last][2] (at most {MAX_PATTERN_DEPTH})"
        )
    };
    let entity = entity.trim();
    let Some(id) = entity
        .strip_prefix('e')
        .unwrap_or(entity)
        .parse::<u64>()
        .ok()
        .filter(|_| !entity.is_empty())
    else {
        return Some(Err(wrong()));
    };
    let mut levels = Vec::new();
    loop {
        if levels.len() >= MAX_PATTERN_DEPTH {
            return Some(Err(wrong()));
        }
        let Some((inner, after)) = rest.split_once(']') else {
            return Some(Err(wrong()));
        };
        let Some(index) = InstanceIndex::parse(inner) else {
            return Some(Err(wrong()));
        };
        levels.push(index);
        let after = after.trim();
        if after.is_empty() {
            break;
        }
        let Some(next) = after.strip_prefix('[') else {
            return Some(Err(wrong()));
        };
        rest = next;
    }
    Some(Ok((EntityId(id), levels)))
}

/// The centre of a circular sketch pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CenterRef {
    /// A point of the sketch (solved like any other).
    Point { point: EntityId },
    /// Fixed coordinates in the sketch's axes.
    At { at: [Expr; 2] },
}

/// The second direction of a grid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondDirection {
    /// The step from one row to the next, in the sketch's axes.
    pub step: [Expr; 2],
    /// Rows, the seed's included: a whole number of at least 1.
    pub count: Expr,
}

/// How the instances of a sketch pattern lie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SketchPatternLayout {
    /// Instance `k` is the seed moved by `k * step` (plus `j * step` of
    /// the second direction in a grid).
    Linear {
        step: [Expr; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        second: Option<SecondDirection>,
    },
    /// The seed is a run of contour from point `from` to point `to`, and
    /// instance `k` starts where instance `k - 1` ends: the step is
    /// `to - from` as solved.
    Chain { from: EntityId, to: EntityId },
    /// Instance `k` is the seed turned by `k * angle` (counter-clockwise)
    /// about `center`. With `chain` the seed runs from the first point to
    /// the second, and the second is the first turned by `angle`.
    Circular {
        center: CenterRef,
        angle: Expr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chain: Option<[EntityId; 2]>,
    },
}

impl SketchPatternLayout {
    pub fn type_name(&self) -> &'static str {
        match self {
            SketchPatternLayout::Linear { second: None, .. } => "linear",
            SketchPatternLayout::Linear {
                second: Some(_), ..
            } => "grid",
            SketchPatternLayout::Chain { .. } => "chain",
            SketchPatternLayout::Circular { chain: None, .. } => "circular",
            SketchPatternLayout::Circular { chain: Some(_), .. } => "circularChain",
        }
    }

    /// The ends of a chained run, if it is one.
    pub fn chain(&self) -> Option<(EntityId, EntityId)> {
        match self {
            SketchPatternLayout::Chain { from, to } => Some((*from, *to)),
            SketchPatternLayout::Circular {
                chain: Some([from, to]),
                ..
            } => Some((*from, *to)),
            _ => None,
        }
    }

    /// True for a grid (two directions).
    pub fn is_grid(&self) -> bool {
        matches!(
            self,
            SketchPatternLayout::Linear {
                second: Some(_),
                ..
            }
        )
    }
}

/// A pattern inside a sketch: the seed entities repeated as rigid images
/// of their solved shape. The instances are not stored: regeneration
/// makes them from the solved seed (docs/design/document.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SketchPattern {
    pub id: PatternId,
    /// The entities repeated; the points of curves come along.
    pub seed: Vec<EntityId>,
    /// Instances, the seed included: a whole number of at least 1.
    pub count: Expr,
    pub layout: SketchPatternLayout,
    /// Whole patterns repeated as part of the seed, with all their
    /// instances and image points: a pattern of a pattern. Each pattern is
    /// in at most one seed, so the patterns of a sketch are trees, at most
    /// [`MAX_PATTERN_DEPTH`] levels deep.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternId>,
    /// Instances of this pattern's own level left out (never instance 0,
    /// the seed), in ascending order. One past the count is kept and has
    /// no effect until the count grows back to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<InstanceIndex>,
}

impl SketchPattern {
    /// Every expression of the pattern with a name that says where it is.
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let id = self.id;
        let mut out = vec![(format!("{id}.count"), &self.count)];
        match &self.layout {
            SketchPatternLayout::Linear { step, second } => {
                out.push((format!("{id}.step.0"), &step[0]));
                out.push((format!("{id}.step.1"), &step[1]));
                if let Some(second) = second {
                    out.push((format!("{id}.step2.0"), &second.step[0]));
                    out.push((format!("{id}.step2.1"), &second.step[1]));
                    out.push((format!("{id}.count2"), &second.count));
                }
            }
            SketchPatternLayout::Chain { .. } => {}
            SketchPatternLayout::Circular { center, angle, .. } => {
                out.push((format!("{id}.angle"), angle));
                if let CenterRef::At { at } = center {
                    out.push((format!("{id}.center.0"), &at[0]));
                    out.push((format!("{id}.center.1"), &at[1]));
                }
            }
        }
        out
    }

    fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = vec![&mut self.count];
        match &mut self.layout {
            SketchPatternLayout::Linear { step, second } => {
                out.extend(step.iter_mut());
                if let Some(second) = second {
                    out.extend(second.step.iter_mut());
                    out.push(&mut second.count);
                }
            }
            SketchPatternLayout::Chain { .. } => {}
            SketchPatternLayout::Circular { center, angle, .. } => {
                out.push(angle);
                if let CenterRef::At { at } = center {
                    out.extend(at.iter_mut());
                }
            }
        }
        out
    }

    /// The point entities the layout itself refers to: the ends of a
    /// chain and the centre.
    pub fn layout_points(&self) -> Vec<EntityId> {
        let mut out = Vec::new();
        if let Some((from, to)) = self.layout.chain() {
            out.extend([from, to]);
        }
        if let SketchPatternLayout::Circular {
            center: CenterRef::Point { point },
            ..
        } = &self.layout
        {
            out.push(*point);
        }
        out
    }
}

/// What deleting an entity or a pattern took with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// Constraints deleted, in the order they were in.
    pub constraints: Vec<ConstraintId>,
    /// Image points deleted, because their seed point or their pattern
    /// went.
    pub images: Vec<EntityId>,
    /// Patterns deleted, because nothing of their seed was left or a
    /// point their layout uses went.
    pub patterns: Vec<PatternId>,
    /// Patterns taken out of the seed of the pattern that held them (the
    /// inner one first), because the inner one was deleted.
    pub unwrapped: Vec<(PatternId, PatternId)>,
}

/// One of the three planes through the world origin. The sketch's own axes
/// follow cyclically, so every sketch frame is right-handed: on `Xy` the
/// sketch x and y are the world X and Y and the normal is Z; on `Yz` they
/// are Y and Z with normal X; on `Zx` they are Z and X with normal Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldPlane {
    #[default]
    Xy,
    Yz,
    Zx,
}

impl WorldPlane {
    pub const ALL: [WorldPlane; 3] = [WorldPlane::Xy, WorldPlane::Yz, WorldPlane::Zx];

    pub fn name(self) -> &'static str {
        match self {
            WorldPlane::Xy => "xy",
            WorldPlane::Yz => "yz",
            WorldPlane::Zx => "zx",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// What a sketch lies on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SketchPlane {
    /// A world plane, moved by `offset` along its normal.
    World { plane: WorldPlane, offset: Expr },
    /// A flat face of a body made earlier in the history, by its
    /// persistent name, moved by `offset` along the face's outward normal.
    /// The sketch's origin and axes are the face's own: for the end cap of
    /// an extrusion those of the extruded sketch, for a side face the
    /// start of its line with x along the line and y along the extrusion.
    Face {
        face: crate::PersistentName,
        offset: Expr,
    },
    /// A datum plane made earlier in the history, moved by `offset` along
    /// its normal. The sketch's origin and axes are the datum's.
    Datum {
        plane: crate::FeatureId,
        offset: Expr,
    },
}

impl SketchPlane {
    /// The offset along the normal.
    pub fn offset(&self) -> &Expr {
        match self {
            SketchPlane::World { offset, .. }
            | SketchPlane::Face { offset, .. }
            | SketchPlane::Datum { offset, .. } => offset,
        }
    }

    pub fn offset_mut(&mut self) -> &mut Expr {
        match self {
            SketchPlane::World { offset, .. }
            | SketchPlane::Face { offset, .. }
            | SketchPlane::Datum { offset, .. } => offset,
        }
    }
}

impl Default for SketchPlane {
    fn default() -> Self {
        SketchPlane::World {
            plane: WorldPlane::default(),
            offset: Expr::new("0"),
        }
    }
}

/// The shape of an entity. Lines, circles, arcs and splines are built on
/// point entities, so two curves that share a corner share one point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Geometry {
    Point {
        x: f64,
        y: f64,
    },
    Line {
        start: EntityId,
        end: EntityId,
    },
    Circle {
        center: EntityId,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` about `center`; both ends
    /// are at the same distance from the centre.
    Arc {
        center: EntityId,
        start: EntityId,
        end: EntityId,
    },
    /// A polynomial B-spline of `degree` whose control points are the
    /// points `control`, in order, on the clamped uniform knots of
    /// [`clamped_uniform_knots`]: it starts at the first control point,
    /// ends at the last, and leaves each along the leg to the next. Its
    /// shape is that of its points, so constraints on them (coincident,
    /// fixed, distance, ...) shape it. With the first point also as the
    /// last it closes on itself.
    ///
    /// Knots and weights are not stored: a later rational or non-uniform
    /// spline adds them as fields that default to what this one means.
    Spline {
        degree: u32,
        control: Vec<EntityId>,
    },
    /// The image of the seed point `of` in instance `index` of the sketch
    /// pattern `pattern`: a point with no coordinates of its own, which
    /// follows the solved seed rigidly. For the solver a derived point
    /// (no unknowns); constraints and lines may use it as a point.
    Image {
        of: EntityId,
        pattern: PatternId,
        index: InstanceIndex,
    },
}

/// The knots of a sketch spline: `degree + 1` zeros, the inner knots evenly
/// spaced, `degree + 1` ones (`count + degree + 1` in all). Empty when
/// there are fewer than `degree + 1` control points, when the degree is 0
/// or when either is beyond [`MAX_SPLINE_DEGREE`] or [`MAX_SPLINE_POINTS`].
pub fn clamped_uniform_knots(degree: u32, count: usize) -> Vec<f64> {
    let Ok(p) = usize::try_from(degree) else {
        return Vec::new();
    };
    if degree == 0 || degree > MAX_SPLINE_DEGREE || count > MAX_SPLINE_POINTS || count <= p {
        return Vec::new();
    }
    let spans = count - p;
    let mut knots = Vec::with_capacity(count + p + 1);
    knots.extend(std::iter::repeat_n(0.0, p + 1));
    // Counts are at most MAX_SPLINE_POINTS, exact as f64.
    #[allow(clippy::cast_precision_loss)]
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    knots
}

impl Geometry {
    pub fn type_name(&self) -> &'static str {
        match self {
            Geometry::Point { .. } => "point",
            Geometry::Line { .. } => "line",
            Geometry::Circle { .. } => "circle",
            Geometry::Arc { .. } => "arc",
            Geometry::Spline { .. } => "spline",
            Geometry::Image { .. } => "image",
        }
    }

    /// The point entities this one is built on.
    pub fn points(&self) -> Vec<EntityId> {
        match self {
            Geometry::Point { .. } | Geometry::Image { .. } => Vec::new(),
            Geometry::Line { start, end } => vec![*start, *end],
            Geometry::Circle { center, .. } => vec![*center],
            Geometry::Arc { center, start, end } => vec![*center, *start, *end],
            Geometry::Spline { control, .. } => control.clone(),
        }
    }

    /// For a curve with two ends, the points it starts and ends at.
    pub fn ends(&self) -> Option<(EntityId, EntityId)> {
        match self {
            Geometry::Point { .. } | Geometry::Circle { .. } | Geometry::Image { .. } => None,
            Geometry::Line { start, end } | Geometry::Arc { start, end, .. } => {
                Some((*start, *end))
            }
            Geometry::Spline { control, .. } => Some((*control.first()?, *control.last()?)),
        }
    }

    /// Why a spline of this degree on these points cannot be, if it cannot:
    /// the rules [`Sketch::add_spline`] and [`Sketch::validate`] share.
    fn spline_fault(degree: u32, control: &[EntityId]) -> Option<String> {
        if degree == 0 || degree > MAX_SPLINE_DEGREE {
            return Some(format!(
                "the degree of a spline is from 1 to {MAX_SPLINE_DEGREE}, got {degree}"
            ));
        }
        if control.len() > MAX_SPLINE_POINTS {
            return Some(format!(
                "a spline has at most {MAX_SPLINE_POINTS} control points, got {}",
                control.len()
            ));
        }
        let needed = usize::try_from(degree).map_or(usize::MAX, |p| p.saturating_add(1));
        if control.len() < needed {
            return Some(format!(
                "a spline of degree {degree} needs at least {needed} control points, got {}",
                control.len()
            ));
        }
        // The same point twice, but for the first as the last: a doubled
        // control point is a kink, which two splines say better.
        let mut seen = BTreeSet::new();
        let last = control.len().saturating_sub(1);
        for (i, id) in control.iter().enumerate() {
            let closes = i == last && control.first() == Some(id) && control.len() > 2;
            if !closes && !seen.insert(*id) {
                return Some(format!(
                    "a spline uses each point once (only its first point may come back as its last), but {id} is there twice"
                ));
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    /// Construction geometry guides constraints but is not part of a
    /// profile.
    #[serde(default)]
    pub construction: bool,
    pub geometry: Geometry,
}

/// Along which direction a distance is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceDirection {
    /// The straight distance.
    #[default]
    Direct,
    /// Along the sketch x axis. Not signed: the points stay on the side of
    /// each other they are on.
    Horizontal,
    /// Along the sketch y axis.
    Vertical,
}

/// A constraint between entities. Which kinds of entity each one accepts
/// is checked by [`Sketch::add_constraint`]; see [`ConstraintKind::roles`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConstraintKind {
    /// Two points at the same place.
    Coincident {
        a: EntityId,
        b: EntityId,
    },
    /// A point on a line (extended), circle or arc (its full circle).
    PointOn {
        point: EntityId,
        curve: EntityId,
    },
    /// A line, or two points, along the sketch x axis.
    Horizontal {
        a: EntityId,
        b: Option<EntityId>,
    },
    /// A line, or two points, along the sketch y axis.
    Vertical {
        a: EntityId,
        b: Option<EntityId>,
    },
    Parallel {
        a: EntityId,
        b: EntityId,
    },
    Perpendicular {
        a: EntityId,
        b: EntityId,
    },
    /// A line and a circle or arc, or two circles or arcs. With a spline:
    /// the spline leaves the end point it shares with the other curve (a
    /// line, an arc or another spline) in that curve's direction there.
    Tangent {
        a: EntityId,
        b: EntityId,
    },
    /// Two lines of equal length, or two circles or arcs of equal radius.
    Equal {
        a: EntityId,
        b: EntityId,
    },
    /// Two circles or arcs with the same centre.
    Concentric {
        a: EntityId,
        b: EntityId,
    },
    /// Points `a` and `b` mirror each other in the line `about`.
    Symmetric {
        a: EntityId,
        b: EntityId,
        about: EntityId,
    },
    /// A point at the middle of a line.
    Midpoint {
        point: EntityId,
        line: EntityId,
    },
    /// A point that stays where it is.
    Fixed {
        point: EntityId,
    },
    /// The length of line `a`, or the distance from point `a` to point or
    /// line `b`.
    Distance {
        a: EntityId,
        b: Option<EntityId>,
        #[serde(default)]
        direction: DistanceDirection,
        value: Expr,
    },
    /// The angle from line `a` to line `b`, counter-clockwise, each line
    /// taken in its direction from start to end.
    Angle {
        a: EntityId,
        b: EntityId,
        value: Expr,
    },
    Radius {
        curve: EntityId,
        value: Expr,
    },
    Diameter {
        curve: EntityId,
        value: Expr,
    },
}

/// The kind of entity a constraint slot accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Point,
    Line,
    /// A circle or an arc.
    Round,
    Spline,
}

impl Role {
    fn of(geometry: &Geometry) -> Role {
        match geometry {
            Geometry::Point { .. } | Geometry::Image { .. } => Role::Point,
            Geometry::Line { .. } => Role::Line,
            Geometry::Circle { .. } | Geometry::Arc { .. } => Role::Round,
            Geometry::Spline { .. } => Role::Spline,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Role::Point => "a point",
            Role::Line => "a line",
            Role::Round => "a circle or arc",
            Role::Spline => "a spline",
        }
    }
}

use Role::{Line, Point, Round, Spline};

impl ConstraintKind {
    /// Every constraint type, by the name used in files and commands.
    pub const TYPES: [&'static str; 16] = [
        "coincident",
        "pointOn",
        "horizontal",
        "vertical",
        "parallel",
        "perpendicular",
        "tangent",
        "equal",
        "concentric",
        "symmetric",
        "midpoint",
        "fixed",
        "distance",
        "angle",
        "radius",
        "diameter",
    ];

    pub fn type_name(&self) -> &'static str {
        match self {
            ConstraintKind::Coincident { .. } => "coincident",
            ConstraintKind::PointOn { .. } => "pointOn",
            ConstraintKind::Horizontal { .. } => "horizontal",
            ConstraintKind::Vertical { .. } => "vertical",
            ConstraintKind::Parallel { .. } => "parallel",
            ConstraintKind::Perpendicular { .. } => "perpendicular",
            ConstraintKind::Tangent { .. } => "tangent",
            ConstraintKind::Equal { .. } => "equal",
            ConstraintKind::Concentric { .. } => "concentric",
            ConstraintKind::Symmetric { .. } => "symmetric",
            ConstraintKind::Midpoint { .. } => "midpoint",
            ConstraintKind::Fixed { .. } => "fixed",
            ConstraintKind::Distance { .. } => "distance",
            ConstraintKind::Angle { .. } => "angle",
            ConstraintKind::Radius { .. } => "radius",
            ConstraintKind::Diameter { .. } => "diameter",
        }
    }

    /// The combinations of entity kinds a constraint type accepts, each in
    /// the order the entities are given. `None` for an unknown type.
    pub fn roles(type_name: &str) -> Option<&'static [&'static [Role]]> {
        Some(match type_name {
            "coincident" => &[&[Point, Point]],
            "pointOn" => &[&[Point, Line], &[Point, Round], &[Point, Spline]],
            "horizontal" | "vertical" => &[&[Line], &[Point, Point]],
            "parallel" | "perpendicular" | "angle" => &[&[Line, Line]],
            "tangent" => &[
                &[Line, Round],
                &[Round, Line],
                &[Round, Round],
                &[Line, Spline],
                &[Spline, Line],
                &[Round, Spline],
                &[Spline, Round],
                &[Spline, Spline],
            ],
            "equal" => &[&[Line, Line], &[Round, Round]],
            "concentric" => &[&[Round, Round]],
            "symmetric" => &[&[Point, Point, Line]],
            "midpoint" => &[&[Point, Line]],
            "fixed" => &[&[Point]],
            "distance" => &[&[Line], &[Point, Point], &[Point, Line]],
            "radius" | "diameter" => &[&[Round]],
            _ => return None,
        })
    }

    /// True for the types that carry a value (a dimension).
    pub fn has_value(type_name: &str) -> bool {
        matches!(type_name, "distance" | "angle" | "radius" | "diameter")
    }

    /// Builds a constraint from its type name, its entities in order and,
    /// for a dimension, its value. Only the number of entities and the
    /// presence of the value are checked here; whether the entities are of
    /// the right kind is checked against a sketch by
    /// [`Sketch::add_constraint`].
    pub fn from_parts(
        type_name: &str,
        entities: &[EntityId],
        value: Option<Expr>,
        direction: DistanceDirection,
    ) -> Result<Self, String> {
        let Some(roles) = Self::roles(type_name) else {
            return Err(format!(
                "unknown constraint type `{type_name}`; the types are: {}",
                Self::TYPES.join(", ")
            ));
        };
        if !roles.iter().any(|r| r.len() == entities.len()) {
            return Err(format!(
                "a {type_name} constraint takes {}, but {} were given",
                describe_roles(roles),
                entities.len()
            ));
        }
        let value = match (Self::has_value(type_name), value) {
            (true, Some(value)) => value,
            (true, None) => return Err(format!("a {type_name} constraint needs a `value`")),
            (false, Some(_)) => {
                return Err(format!("a {type_name} constraint takes no `value`"));
            }
            (false, None) => Expr::default(),
        };
        if direction != DistanceDirection::Direct && type_name != "distance" {
            return Err(format!("a {type_name} constraint takes no `direction`"));
        }
        let at = |i: usize| entities.get(i).copied();
        let need = |i: usize| at(i).ok_or_else(|| format!("entity {i} is missing"));
        let (a, b) = (need(0)?, at(1));
        let second = || need(1);
        Ok(match type_name {
            "coincident" => ConstraintKind::Coincident { a, b: second()? },
            "pointOn" => ConstraintKind::PointOn {
                point: a,
                curve: second()?,
            },
            "horizontal" => ConstraintKind::Horizontal { a, b },
            "vertical" => ConstraintKind::Vertical { a, b },
            "parallel" => ConstraintKind::Parallel { a, b: second()? },
            "perpendicular" => ConstraintKind::Perpendicular { a, b: second()? },
            "tangent" => ConstraintKind::Tangent { a, b: second()? },
            "equal" => ConstraintKind::Equal { a, b: second()? },
            "concentric" => ConstraintKind::Concentric { a, b: second()? },
            "symmetric" => ConstraintKind::Symmetric {
                a,
                b: second()?,
                about: need(2)?,
            },
            "midpoint" => ConstraintKind::Midpoint {
                point: a,
                line: second()?,
            },
            "fixed" => ConstraintKind::Fixed { point: a },
            "distance" => ConstraintKind::Distance {
                a,
                b,
                direction,
                value,
            },
            "angle" => ConstraintKind::Angle {
                a,
                b: second()?,
                value,
            },
            "radius" => ConstraintKind::Radius { curve: a, value },
            "diameter" => ConstraintKind::Diameter { curve: a, value },
            other => return Err(format!("unknown constraint type `{other}`")),
        })
    }

    /// The entities the constraint refers to, in the order they are given
    /// to [`ConstraintKind::from_parts`].
    pub fn entities(&self) -> Vec<EntityId> {
        match *self {
            ConstraintKind::Coincident { a, b }
            | ConstraintKind::Parallel { a, b }
            | ConstraintKind::Perpendicular { a, b }
            | ConstraintKind::Tangent { a, b }
            | ConstraintKind::Equal { a, b }
            | ConstraintKind::Concentric { a, b }
            | ConstraintKind::Angle { a, b, .. } => vec![a, b],
            ConstraintKind::PointOn { point, curve } => vec![point, curve],
            ConstraintKind::Horizontal { a, b }
            | ConstraintKind::Vertical { a, b }
            | ConstraintKind::Distance { a, b, .. } => std::iter::once(a).chain(b).collect(),
            ConstraintKind::Symmetric { a, b, about } => vec![a, b, about],
            ConstraintKind::Midpoint { point, line } => vec![point, line],
            ConstraintKind::Fixed { point } => vec![point],
            ConstraintKind::Radius { curve, .. } | ConstraintKind::Diameter { curve, .. } => {
                vec![curve]
            }
        }
    }

    /// The dimension's expression, for the types that have one.
    pub fn value(&self) -> Option<&Expr> {
        match self {
            ConstraintKind::Distance { value, .. }
            | ConstraintKind::Angle { value, .. }
            | ConstraintKind::Radius { value, .. }
            | ConstraintKind::Diameter { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn value_mut(&mut self) -> Option<&mut Expr> {
        match self {
            ConstraintKind::Distance { value, .. }
            | ConstraintKind::Angle { value, .. }
            | ConstraintKind::Radius { value, .. }
            | ConstraintKind::Diameter { value, .. } => Some(value),
            _ => None,
        }
    }
}

fn describe_roles(roles: &[&[Role]]) -> String {
    roles
        .iter()
        .map(|combination| {
            combination
                .iter()
                .map(|r| r.noun())
                .collect::<Vec<_>>()
                .join(" and ")
        })
        .collect::<Vec<_>>()
        .join(", or ")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub id: ConstraintId,
    pub kind: ConstraintKind,
}

/// What can be wrong with a request to change a sketch.
#[derive(Clone, Debug, PartialEq)]
pub enum SketchError {
    UnknownEntity(EntityId),
    UnknownConstraint(ConstraintId),
    /// An entity of the wrong kind, a number that is not finite, a cap
    /// that is reached: the text says which.
    Invalid(String),
    /// The entity cannot be deleted while these are built on it.
    InUse {
        entity: EntityId,
        by: Vec<EntityId>,
    },
}

impl fmt::Display for SketchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SketchError::UnknownEntity(id) => write!(f, "the sketch has no entity {id}"),
            SketchError::UnknownConstraint(id) => write!(f, "the sketch has no constraint {id}"),
            SketchError::Invalid(what) => f.write_str(what),
            SketchError::InUse { entity, by } => {
                let users: Vec<String> = by.iter().take(10).map(|e| e.to_string()).collect();
                write!(
                    f,
                    "{entity} cannot be deleted: {} built on it; delete those first",
                    match users.as_slice() {
                        [one] => format!("{one} is"),
                        _ => format!("{} are", users.join(", ")),
                    }
                )
            }
        }
    }
}

impl std::error::Error for SketchError {}

fn invalid<T>(what: impl Into<String>) -> Result<T, SketchError> {
    Err(SketchError::Invalid(what.into()))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane: SketchPlane,
    /// In the order they were drawn.
    pub entities: Vec<Entity>,
    /// In the order they were added: the solver blames the later one when
    /// two disagree.
    pub constraints: Vec<Constraint>,
    /// The id the next entity gets.
    pub next_entity_id: u64,
    /// The id the next constraint gets.
    pub next_constraint_id: u64,
    /// Patterns of seed entities, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<SketchPattern>,
    /// The id the next pattern gets.
    #[serde(default = "first_id", skip_serializing_if = "is_first_id")]
    pub next_pattern_id: u64,
    /// Where the image points lay in the stored solution, as the points'
    /// coordinates are the stored solution of the points: when an edit
    /// moves an image (a count, step or angle of its pattern), the solver
    /// carries the sketch along from there, so what hangs on the image
    /// follows it. Written by the solve with the coordinates; an image
    /// that is not listed has nothing to be carried from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_places: Vec<ImagePlace>,
}

/// Where an image point lay in the stored solution of its sketch, and by
/// what map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePlace {
    pub entity: EntityId,
    pub at: [f64; 2],
    /// The numbers of the map that put it there, when known: the solver
    /// carries the image through its pattern's own family from them (the
    /// angle of a ring goes from its old value to its new one), and in a
    /// straight line from `at` without them or when the pattern's kind
    /// changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<PlaceMap>,
}

/// The numbers of the map an image point was placed by: what changes when
/// a count, step or angle of its pattern does. The points the map uses
/// (the ends of a chained run, a centre point) are the pattern's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlaceMap {
    /// A row or a grid: the seed point moved by `offset`.
    Shift { offset: [f64; 2] },
    /// A chained run: instance `k`.
    Chain { k: f64 },
    /// A turn by `angle` (radians) about the pattern's centre point, or
    /// about `at` when it has none.
    Turn { angle: f64, at: [f64; 2] },
}

impl PlaceMap {
    fn is_finite(&self) -> bool {
        match *self {
            PlaceMap::Shift { offset } => offset.iter().all(|v| v.is_finite()),
            PlaceMap::Chain { k } => k.is_finite(),
            PlaceMap::Turn { angle, at } => angle.is_finite() && at.iter().all(|v| v.is_finite()),
        }
    }
}

impl ImagePlace {
    fn is_finite(&self) -> bool {
        self.at.iter().all(|v| v.is_finite()) && self.map.is_none_or(|m| m.is_finite())
    }
}

fn first_id() -> u64 {
    1
}

fn is_first_id(id: &u64) -> bool {
    *id == 1
}

impl Default for Sketch {
    fn default() -> Self {
        Self::new(SketchPlane::default())
    }
}

impl Sketch {
    pub fn new(plane: SketchPlane) -> Self {
        Self {
            plane,
            entities: Vec::new(),
            constraints: Vec::new(),
            next_entity_id: 1,
            next_constraint_id: 1,
            patterns: Vec::new(),
            next_pattern_id: 1,
            image_places: Vec::new(),
        }
    }

    pub fn entity(&self, id: EntityId) -> Result<&Entity, SketchError> {
        self.entities
            .iter()
            .find(|e| e.id == id)
            .ok_or(SketchError::UnknownEntity(id))
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Result<&mut Entity, SketchError> {
        self.entities
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(SketchError::UnknownEntity(id))
    }

    pub fn constraint(&self, id: ConstraintId) -> Result<&Constraint, SketchError> {
        self.constraints
            .iter()
            .find(|c| c.id == id)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    pub fn constraint_mut(&mut self, id: ConstraintId) -> Result<&mut Constraint, SketchError> {
        self.constraints
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    /// Checks that `id` is a point: one with coordinates, or an image
    /// point of a pattern.
    pub fn point_entity(&self, id: EntityId) -> Result<(), SketchError> {
        match &self.entity(id)?.geometry {
            Geometry::Point { .. } | Geometry::Image { .. } => Ok(()),
            other => invalid(format!("{id} is a {}, not a point", other.type_name())),
        }
    }

    /// True for an image point.
    pub fn is_image(&self, id: EntityId) -> bool {
        self.entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Image { .. }))
    }

    /// The coordinates of a point entity.
    pub fn point(&self, id: EntityId) -> Result<(f64, f64), SketchError> {
        match &self.entity(id)?.geometry {
            Geometry::Point { x, y } => Ok((*x, *y)),
            other => invalid(format!("{id} is a {}, not a point", other.type_name())),
        }
    }

    fn push(&mut self, geometry: Geometry, construction: bool) -> Result<EntityId, SketchError> {
        if self.entities.len() >= MAX_SKETCH_ENTITIES {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_ENTITIES} entities"
            ));
        }
        for point in geometry.points() {
            self.point_entity(point)?;
        }
        if let Some(fault) = self.image_fault(&geometry) {
            return invalid(fault);
        }
        if self.next_entity_id > MAX_ENTITY_ID {
            return invalid("the sketch has run out of entity ids");
        }
        let id = EntityId(self.next_entity_id);
        self.next_entity_id = self
            .next_entity_id
            .checked_add(1)
            .ok_or_else(|| SketchError::Invalid("the sketch has run out of entity ids".into()))?;
        self.entities.push(Entity {
            id,
            construction,
            geometry,
        });
        Ok(id)
    }

    pub fn add_point(&mut self, x: f64, y: f64) -> Result<EntityId, SketchError> {
        if !(x.is_finite() && y.is_finite()) {
            return invalid("the coordinates of a point must be finite numbers");
        }
        self.push(Geometry::Point { x, y }, false)
    }

    /// A line between two existing points.
    pub fn add_line(
        &mut self,
        start: EntityId,
        end: EntityId,
        construction: bool,
    ) -> Result<EntityId, SketchError> {
        if start == end {
            return invalid(format!(
                "a line needs two different points, got {start} twice"
            ));
        }
        self.push(Geometry::Line { start, end }, construction)
    }

    pub fn add_circle(
        &mut self,
        center: EntityId,
        radius: f64,
        construction: bool,
    ) -> Result<EntityId, SketchError> {
        if !(radius.is_finite() && radius > 0.0) {
            return invalid(format!(
                "the radius of a circle must be a positive number, got {radius}"
            ));
        }
        self.push(Geometry::Circle { center, radius }, construction)
    }

    /// An arc, counter-clockwise from `start` to `end` about `center`.
    pub fn add_arc(
        &mut self,
        center: EntityId,
        start: EntityId,
        end: EntityId,
        construction: bool,
    ) -> Result<EntityId, SketchError> {
        if center == start || center == end || start == end {
            return invalid("an arc needs three different points: centre, start and end");
        }
        self.push(Geometry::Arc { center, start, end }, construction)
    }

    /// A spline of `degree` with the existing points `control` as its
    /// control points, on clamped uniform knots. Closed when the first
    /// point is also the last.
    pub fn add_spline(
        &mut self,
        degree: u32,
        control: Vec<EntityId>,
        construction: bool,
    ) -> Result<EntityId, SketchError> {
        if let Some(fault) = Geometry::spline_fault(degree, &control) {
            return invalid(fault);
        }
        self.push(Geometry::Spline { degree, control }, construction)
    }

    /// Checks that the constraint's entities exist, are different from each
    /// other and are of a kind the constraint accepts.
    pub fn check_constraint(&self, kind: &ConstraintKind) -> Result<(), SketchError> {
        let type_name = kind.type_name();
        let entities = kind.entities();
        let mut given = Vec::with_capacity(entities.len());
        for id in &entities {
            given.push(Role::of(&self.entity(*id)?.geometry));
        }
        let mut seen = BTreeSet::new();
        if let Some(twice) = entities.iter().find(|id| !seen.insert(**id)) {
            return invalid(format!(
                "a {type_name} constraint needs different entities, got {twice} twice"
            ));
        }
        let roles = ConstraintKind::roles(type_name).unwrap_or(&[]);
        if !roles.contains(&given.as_slice()) {
            let got: Vec<String> = entities
                .iter()
                .zip(&given)
                .map(|(id, role)| format!("{id} ({})", role.noun()))
                .collect();
            let unsupported = if given.contains(&Role::Spline) {
                " (not supported for splines yet)"
            } else {
                ""
            };
            return invalid(format!(
                "a {type_name} constraint takes {}, got {}{unsupported}",
                describe_roles(roles),
                got.join(" and ")
            ));
        }

        // Images of one pattern, or of one tree of patterns, are tied to
        // each other by the patterns: a constraint between them only is
        // redundant or conflicting by construction.
        let roots: BTreeSet<Option<PatternId>> = entities
            .iter()
            .map(|id| match self.entity(*id).map(|e| &e.geometry) {
                Ok(Geometry::Image { pattern, .. }) => {
                    self.pattern_levels(*pattern).last().copied()
                }
                _ => None,
            })
            .collect();
        if entities.len() > 1
            && let [Some(root)] = roots.into_iter().collect::<Vec<_>>().as_slice()
        {
            return invalid(if self.inner_patterns(*root).is_empty() {
                format!(
                    "a {type_name} constraint between images of one pattern ({root}) only is not supported yet: the pattern ties them already; constrain the seed instead"
                )
            } else {
                format!(
                    "a {type_name} constraint between images of one tree of patterns ({root} and the patterns in it) only is not supported yet: the patterns tie them already; constrain the seed instead"
                )
            });
        }

        if let ConstraintKind::Distance {
            b: Some(b),
            direction,
            ..
        } = kind
            && *direction != DistanceDirection::Direct
            && Role::of(&self.entity(*b)?.geometry) != Role::Point
        {
            return invalid(
                "a horizontal or vertical distance is measured between two points or along one line",
            );
        }
        Ok(())
    }

    /// For a tangency that involves a spline: the end points at which the
    /// two curves meet, `(end of a, end of b)`. Ends meet when they are one
    /// point entity or are tied by a coincident constraint. `None` when
    /// the curves share no end (or one of them has no ends).
    pub fn tangent_at(&self, a: EntityId, b: EntityId) -> Option<(EntityId, EntityId)> {
        let ends = |id: EntityId| {
            let geometry = &self.entity(id).ok()?.geometry;
            let (start, end) = geometry.ends()?;
            Some([start, end])
        };
        let (ends_a, ends_b) = (ends(a)?, ends(b)?);
        let tied = |p: EntityId, q: EntityId| {
            p == q
                || self.constraints.iter().any(|c| {
                    matches!(c.kind, ConstraintKind::Coincident { a, b }
                        if (a, b) == (p, q) || (a, b) == (q, p))
                })
        };
        ends_a
            .into_iter()
            .flat_map(|p| ends_b.into_iter().map(move |q| (p, q)))
            .find(|&(p, q)| tied(p, q))
    }

    pub fn add_constraint(&mut self, kind: ConstraintKind) -> Result<ConstraintId, SketchError> {
        if self.constraints.len() >= MAX_SKETCH_CONSTRAINTS {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_CONSTRAINTS} constraints"
            ));
        }
        self.check_constraint(&kind)?;
        // Checked when the constraint is made, not as a rule of the sketch:
        // the coincident constraint that ties the ends may be deleted
        // later, and the sketch then fails to solve with this reason
        // instead of the file failing to load.
        if let ConstraintKind::Tangent { a, b } = &kind
            && [*a, *b].iter().any(|id| {
                matches!(
                    self.entity(*id).map(|e| &e.geometry),
                    Ok(Geometry::Spline { .. })
                )
            })
            && self.tangent_at(*a, *b).is_none()
        {
            return invalid(format!(
                "a spline is tangent to another curve at an end point the two share, but {a} and {b} share none: build both on one point, or make their ends coincident first"
            ));
        }
        let id = ConstraintId(self.next_constraint_id);
        self.next_constraint_id = self.next_constraint_id.checked_add(1).ok_or_else(|| {
            SketchError::Invalid("the sketch has run out of constraint ids".into())
        })?;
        self.constraints.push(Constraint { id, kind });
        Ok(id)
    }

    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        let index = self
            .constraints
            .iter()
            .position(|c| c.id == id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        Ok(self.constraints.remove(index))
    }

    /// Removes an entity and the constraints on it, and returns the ids of
    /// those constraints. Refused while a line, circle, arc or spline is
    /// built on it.
    pub fn remove_entity(&mut self, id: EntityId) -> Result<Vec<ConstraintId>, SketchError> {
        self.remove_entity_reporting(id).map(|r| r.constraints)
    }

    /// Removes an entity and what goes with it: the constraints on it; it
    /// leaves the seed of its pattern, and a pattern left without seed, or
    /// without a point its layout uses, goes too; image points whose seed
    /// point or pattern went go, with their constraints. Refused while a
    /// line, circle, arc or spline is built on the entity or on an image
    /// point that would go. Nothing changes when it is refused.
    pub fn remove_entity_reporting(&mut self, id: EntityId) -> Result<Removal, SketchError> {
        let mut next = self.clone();
        let constraints = next.remove_one(id)?;
        let mut removal = Removal {
            constraints,
            ..Removal::default()
        };
        for pattern in &mut next.patterns {
            pattern.seed.retain(|seed| *seed != id);
        }
        next.settle(&mut removal, &[id])?;
        *self = next;
        Ok(removal)
    }

    /// Removes the pattern and its image points with their constraints;
    /// a pattern that held it in its seed holds it no more (reported in
    /// [`Removal::unwrapped`]), and the patterns it held are patterns of
    /// their own again. Refused while a curve is built on one of its image
    /// points.
    pub fn remove_pattern(&mut self, id: PatternId) -> Result<Removal, SketchError> {
        let mut next = self.clone();
        let index = next
            .patterns
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| SketchError::Invalid(format!("the sketch has no pattern {id}")))?;
        next.patterns.remove(index);
        let mut removal = Removal {
            patterns: vec![id],
            ..Removal::default()
        };
        next.unwrap_pattern(id, &mut removal);
        next.settle(&mut removal, &[])?;
        *self = next;
        Ok(removal)
    }

    /// Takes the pattern `id` out of the seed of the pattern holding it.
    fn unwrap_pattern(&mut self, id: PatternId, removal: &mut Removal) {
        for pattern in &mut self.patterns {
            if pattern.patterns.contains(&id) {
                pattern.patterns.retain(|q| *q != id);
                removal.unwrapped.push((id, pattern.id));
            }
        }
    }

    /// After a deletion: patterns that are no longer whole (nothing left
    /// in their seed, a point their layout uses gone (`gone`), a rule
    /// broken) go, and image points whose pattern or seed point went go
    /// with their constraints, round after round until nothing more goes:
    /// an image may be the end of the run of the pattern around its own.
    fn settle(&mut self, removal: &mut Removal, gone: &[EntityId]) -> Result<(), SketchError> {
        // Every round but the last removes a pattern or an image point.
        let rounds = self
            .patterns
            .len()
            .saturating_add(self.entities.len())
            .saturating_add(1);
        for _ in 0..rounds {
            let broken: Vec<PatternId> = self
                .patterns
                .iter()
                .filter(|p| {
                    (p.seed.is_empty() && p.patterns.is_empty())
                        || p.layout_points().iter().any(|point| gone.contains(point))
                        || self.check_pattern(p, Some(p.id)).is_err()
                })
                .map(|p| p.id)
                .collect();
            for id in &broken {
                self.patterns.retain(|p| p.id != *id);
                removal.patterns.push(*id);
                self.unwrap_pattern(*id, removal);
            }
            let lost = self.lost_images();
            for image in &lost {
                let constraints = self.remove_one(*image)?;
                removal.constraints.extend(constraints);
                removal.images.push(*image);
            }
            if broken.is_empty() && lost.is_empty() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// The image points whose pattern is gone or whose seed point is no
    /// longer repeated by it.
    fn lost_images(&self) -> Vec<EntityId> {
        let mut points: std::collections::BTreeMap<PatternId, BTreeSet<EntityId>> =
            std::collections::BTreeMap::new();
        let mut lost = Vec::new();
        for entity in &self.entities {
            let Geometry::Image { of, pattern, .. } = &entity.geometry else {
                continue;
            };
            let Some(found) = self.pattern(*pattern) else {
                lost.push(entity.id);
                continue;
            };
            let tree = points
                .entry(*pattern)
                .or_insert_with(|| self.tree_points(found));
            if !tree.contains(of) {
                lost.push(entity.id);
            }
        }
        lost
    }
    /// Removes one entity and the constraints on it; refused while a
    /// curve is built on it.
    fn remove_one(&mut self, id: EntityId) -> Result<Vec<ConstraintId>, SketchError> {
        let index = self
            .entities
            .iter()
            .position(|e| e.id == id)
            .ok_or(SketchError::UnknownEntity(id))?;
        let by: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|e| e.geometry.points().contains(&id))
            .map(|e| e.id)
            .collect();
        if !by.is_empty() {
            return Err(SketchError::InUse { entity: id, by });
        }
        self.entities.remove(index);
        if let Ok(at) = self.image_places.binary_search_by_key(&id, |p| p.entity) {
            self.image_places.remove(at);
        }
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.constraints)
            .into_iter()
            .partition(|c| c.kind.entities().contains(&id));
        self.constraints = kept;
        Ok(gone.into_iter().map(|c| c.id).collect())
    }

    /// Every expression of the sketch with a name that says where it is.
    pub fn exprs(&self) -> Vec<(String, &Expr)> {
        let mut out = vec![("plane.offset".to_string(), self.plane.offset())];
        out.extend(
            self.constraints
                .iter()
                .filter_map(|c| Some((format!("{}.value", c.id), c.kind.value()?))),
        );
        for pattern in &self.patterns {
            out.extend(pattern.exprs());
        }
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = vec![self.plane.offset_mut()];
        out.extend(
            self.constraints
                .iter_mut()
                .filter_map(|c| c.kind.value_mut()),
        );
        for pattern in &mut self.patterns {
            out.extend(pattern.exprs_mut());
        }
        out
    }

    pub fn pattern(&self, id: PatternId) -> Option<&SketchPattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    /// The pattern that holds pattern `id` in its seed, if one does.
    pub fn parent_pattern(&self, id: PatternId) -> Option<PatternId> {
        self.patterns
            .iter()
            .find(|p| p.patterns.contains(&id))
            .map(|p| p.id)
    }

    /// Pattern `id` and the patterns around it, the innermost first: the
    /// levels of the names of what it makes. Bounded, and a cycle (only a
    /// hostile file has one, and [`Sketch::validate`] refuses it) ends it.
    pub fn pattern_levels(&self, id: PatternId) -> Vec<PatternId> {
        let mut levels = vec![id];
        let mut at = id;
        while levels.len() <= MAX_PATTERN_DEPTH {
            match self.parent_pattern(at) {
                Some(parent) if !levels.contains(&parent) => {
                    levels.push(parent);
                    at = parent;
                }
                _ => break,
            }
        }
        levels
    }

    /// The patterns in the seed of pattern `id`, in theirs, and so on.
    pub fn inner_patterns(&self, id: PatternId) -> BTreeSet<PatternId> {
        let mut found = BTreeSet::new();
        let mut stack: Vec<PatternId> = self
            .pattern(id)
            .map(|p| p.patterns.clone())
            .unwrap_or_default();
        while let Some(next) = stack.pop() {
            if found.insert(next)
                && let Some(pattern) = self.pattern(next)
            {
                stack.extend(pattern.patterns.iter().copied());
            }
        }
        found
    }

    /// Levels of the tree under the pattern `id`, its own included; past
    /// `MAX_PATTERN_DEPTH` (only in a cycle or a broken file) it stops
    /// counting there.
    fn pattern_height(&self, id: PatternId) -> usize {
        fn height(sketch: &Sketch, id: PatternId, depth: usize) -> usize {
            if depth > MAX_PATTERN_DEPTH {
                return depth;
            }
            let below = sketch
                .pattern(id)
                .map(|p| {
                    p.patterns
                        .iter()
                        .map(|q| height(sketch, *q, depth + 1))
                        .max()
                        .unwrap_or(depth)
                })
                .unwrap_or(depth);
            below.max(depth)
        }
        height(self, id, 1)
    }

    /// The points a pattern repeats: the seed's points and the points its
    /// curves are built on.
    pub fn seed_points(&self, pattern: &SketchPattern) -> BTreeSet<EntityId> {
        let mut points = BTreeSet::new();
        for id in &pattern.seed {
            if let Ok(entity) = self.entity(*id) {
                match &entity.geometry {
                    Geometry::Point { .. } => {
                        points.insert(*id);
                    }
                    other => points.extend(other.points()),
                }
            }
        }
        points
    }

    /// Every point a pattern repeats: those of its own seed and those of
    /// the patterns in it, with their image points (which move with the
    /// pattern around them as rigidly as the rest).
    pub fn tree_points(&self, pattern: &SketchPattern) -> BTreeSet<EntityId> {
        let inner = self.inner_patterns(pattern.id);
        let mut inner: BTreeSet<PatternId> = inner;
        inner.extend(pattern.patterns.iter().copied());
        self.points_with(pattern, &inner)
    }

    /// [`Sketch::tree_points`] of `pattern` with the patterns `inner` in
    /// it at every level.
    fn points_with(
        &self,
        pattern: &SketchPattern,
        inner: &BTreeSet<PatternId>,
    ) -> BTreeSet<EntityId> {
        let mut points = self.seed_points(pattern);
        for q in inner {
            if let Some(found) = self.pattern(*q) {
                points.extend(self.seed_points(found));
            }
        }
        if !inner.is_empty() {
            points.extend(self.entities.iter().filter_map(|e| match &e.geometry {
                Geometry::Image { pattern, .. } if inner.contains(pattern) => Some(e.id),
                _ => None,
            }));
        }
        points
    }

    /// The pattern whose seed has `entity` (a seed entity, not a point of a
    /// seed curve).
    pub fn pattern_of_seed(&self, entity: EntityId) -> Option<&SketchPattern> {
        self.patterns.iter().find(|p| p.seed.contains(&entity))
    }

    /// Why a curve cannot be built on the image points it would use.
    fn image_fault(&self, geometry: &Geometry) -> Option<String> {
        let images: Vec<EntityId> = geometry
            .points()
            .into_iter()
            .filter(|p| self.is_image(*p))
            .collect();
        match (geometry, images.first()) {
            (_, None) | (Geometry::Line { .. }, _) => None,
            (other, Some(image)) => Some(format!(
                "a {} built on the image point {image} is not supported yet: only a line may end on an image point",
                other.type_name()
            )),
        }
    }

    /// Checks a pattern against the sketch: its seed exists, is not an
    /// image, is built on image points only of the patterns in it, and is
    /// the seed of no other pattern; the patterns in its seed exist, are in
    /// no other seed, do not hold it and nest at most
    /// [`MAX_PATTERN_DEPTH`] deep; its layout uses points it repeats (a
    /// chain) or of the sketch (a centre); its skips are its own instances,
    /// in order, not instance 0 and not of a chained run. `own` is the
    /// pattern it replaces, if any.
    pub fn check_pattern(
        &self,
        pattern: &SketchPattern,
        own: Option<PatternId>,
    ) -> Result<(), SketchError> {
        let id = pattern.id;
        if pattern.seed.is_empty() && pattern.patterns.is_empty() {
            return invalid(format!(
                "{id}: a pattern repeats at least one entity or pattern"
            ));
        }
        if pattern.seed.len() > MAX_SKETCH_ENTITIES {
            return invalid(format!(
                "{id}: a pattern repeats at most {MAX_SKETCH_ENTITIES} entities"
            ));
        }
        if pattern.patterns.len() > MAX_SKETCH_PATTERNS {
            return invalid(format!(
                "{id}: a pattern repeats at most {MAX_SKETCH_PATTERNS} patterns"
            ));
        }
        let mut held = BTreeSet::new();
        for q in &pattern.patterns {
            if *q == id {
                return invalid(format!("{id}: a pattern cannot repeat itself"));
            }
            if !held.insert(*q) {
                return invalid(format!("{id}: {q} is in the seed twice"));
            }
            if self.pattern(*q).is_none() {
                return invalid(format!("{id}: the sketch has no pattern {q}"));
            }
            if let Some(other) = self
                .patterns
                .iter()
                .find(|p| Some(p.id) != own && p.id != id && p.patterns.contains(q))
            {
                return invalid(format!(
                    "{id}: {q} is in the seed of {} already; a pattern is in the seed of one pattern",
                    other.id
                ));
            }
            if self.inner_patterns(*q).contains(&id) {
                return invalid(format!(
                    "{id}: {q} repeats {id} already; patterns that repeat each other make a cycle"
                ));
            }
        }
        let below = pattern
            .patterns
            .iter()
            .map(|q| self.pattern_height(*q))
            .max()
            .unwrap_or(0);
        let above = self.pattern_levels(id).len().saturating_sub(1);
        let levels = above.saturating_add(below).saturating_add(1);
        if levels > MAX_PATTERN_DEPTH {
            return invalid(format!(
                "{id}: patterns nest at most {MAX_PATTERN_DEPTH} levels deep, and this would make {levels}"
            ));
        }
        // The patterns in this one, at every level, as it is now.
        let mut inner = held.clone();
        for q in &held {
            inner.extend(self.inner_patterns(*q));
        }
        let mut seen = BTreeSet::new();
        for seed in &pattern.seed {
            if !seen.insert(*seed) {
                return invalid(format!("{id}: {seed} is in the seed twice"));
            }
            let entity = self.entity(*seed)?;
            if matches!(entity.geometry, Geometry::Image { .. }) {
                return invalid(format!(
                    "{id}: {seed} is an image point; a pattern of a pattern's images is not supported yet (put the pattern itself in the seed)"
                ));
            }
            for point in entity.geometry.points() {
                if let Ok(Geometry::Image { pattern: of, .. }) =
                    self.entity(point).map(|e| &e.geometry)
                    && !inner.contains(of)
                {
                    return invalid(format!(
                        "{id}: {seed} is built on the image point {point} of {of}, which is not in the seed; a seed is built on image points only of the patterns in it"
                    ));
                }
            }
            if let Some(other) = self
                .patterns
                .iter()
                .find(|p| Some(p.id) != own && p.id != id && p.seed.contains(seed))
            {
                return invalid(format!(
                    "{id}: {seed} is in the seed of {} already; an entity is the seed of one pattern",
                    other.id
                ));
            }
        }
        let points = self.points_with(pattern, &inner);
        if let Some((from, to)) = pattern.layout.chain() {
            if from == to {
                return invalid(format!(
                    "{id}: a chain runs from one point to another, got {from} twice"
                ));
            }
            for end in [from, to] {
                if !points.contains(&end) {
                    let shown: Vec<String> =
                        points.iter().take(20).map(ToString::to_string).collect();
                    return invalid(format!(
                        "{id}: the chain end {end} is not a point of the seed ({}{})",
                        shown.join(", "),
                        if points.len() > shown.len() {
                            ", …"
                        } else {
                            ""
                        }
                    ));
                }
            }
        }
        if let SketchPatternLayout::Circular {
            center: CenterRef::Point { point },
            ..
        } = &pattern.layout
        {
            match &self.entity(*point)?.geometry {
                Geometry::Point { .. } => {}
                Geometry::Image { .. } => {
                    return invalid(format!(
                        "{id}: the centre {point} is an image point, which is not supported yet"
                    ));
                }
                other => {
                    return invalid(format!(
                        "{id}: the centre {point} is a {}, not a point",
                        other.type_name()
                    ));
                }
            }
        }
        self.check_skip(pattern)
    }

    /// The rules for the instances a pattern skips.
    fn check_skip(&self, pattern: &SketchPattern) -> Result<(), SketchError> {
        let id = pattern.id;
        if pattern.skip.is_empty() {
            return Ok(());
        }
        if pattern.layout.chain().is_some() {
            return invalid(format!(
                "{id}: skipping instances of a chained run or a ring is not supported yet: a skip would leave a gap with two open ends"
            ));
        }
        if pattern.skip.len() > MAX_PATTERN_INSTANCES {
            return invalid(format!(
                "{id}: a pattern skips at most {MAX_PATTERN_INSTANCES} instances"
            ));
        }
        let grid = pattern.layout.is_grid();
        let mut last: Option<(u32, u32)> = None;
        for index in &pattern.skip {
            let key = match (*index, grid) {
                (InstanceIndex::At(k), false) => (k, 0),
                (InstanceIndex::Grid([i, j]), true) => (i, j),
                (InstanceIndex::Last, _) => {
                    return invalid(format!(
                        "{id}: a skipped instance is given by its number, not as last"
                    ));
                }
                (InstanceIndex::At(k), true) => {
                    return invalid(format!(
                        "{id} is a grid: a skipped instance of it has two numbers, as [{k},0]"
                    ));
                }
                (InstanceIndex::Grid([i, j]), false) => {
                    return invalid(format!(
                        "{id} goes one way: a skipped instance of it has one number, not [{i},{j}]"
                    ));
                }
            };
            if key == (0, 0) {
                return invalid(format!(
                    "{id}: instance 0 is the seed, which is drawn and solved; it cannot be skipped"
                ));
            }
            if last.is_some_and(|l| l >= key) {
                return invalid(format!(
                    "{id}: the skipped instances are listed in ascending order, each once ({index})"
                ));
            }
            last = Some(key);
        }
        Ok(())
    }

    /// Adds a pattern of `seed` and returns its id.
    pub fn add_pattern(
        &mut self,
        seed: Vec<EntityId>,
        count: Expr,
        layout: SketchPatternLayout,
    ) -> Result<PatternId, SketchError> {
        self.add_nested_pattern(seed, Vec::new(), count, layout, Vec::new())
    }

    /// Adds a pattern of `seed` and of the whole patterns `patterns`,
    /// without the instances `skip`, and returns its id.
    pub fn add_nested_pattern(
        &mut self,
        seed: Vec<EntityId>,
        patterns: Vec<PatternId>,
        count: Expr,
        layout: SketchPatternLayout,
        skip: Vec<InstanceIndex>,
    ) -> Result<PatternId, SketchError> {
        if self.patterns.len() >= MAX_SKETCH_PATTERNS {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_PATTERNS} patterns"
            ));
        }
        let id = PatternId(self.next_pattern_id);
        let pattern = SketchPattern {
            id,
            seed,
            count,
            layout,
            patterns,
            skip,
        };
        self.check_pattern(&pattern, None)?;
        self.next_pattern_id = self
            .next_pattern_id
            .checked_add(1)
            .ok_or_else(|| SketchError::Invalid("the sketch has run out of pattern ids".into()))?;
        self.patterns.push(pattern);
        Ok(id)
    }

    /// Gives pattern `id` a new seed, count and layout, keeping the
    /// patterns in its seed and its skips; see
    /// [`Sketch::replace_nested_pattern`].
    pub fn replace_pattern(
        &mut self,
        id: PatternId,
        seed: Vec<EntityId>,
        count: Expr,
        layout: SketchPatternLayout,
    ) -> Result<Removal, SketchError> {
        let Some(old) = self.pattern(id) else {
            return invalid(format!("the sketch has no pattern {id}"));
        };
        let (patterns, skip) = (old.patterns.clone(), old.skip.clone());
        self.replace_nested_pattern(id, seed, patterns, count, layout, skip)
    }

    /// Gives pattern `id` a new seed, inner patterns, count, layout and
    /// skips; it stays in the seed of the pattern that holds it. Image
    /// points whose seed point is no longer repeated go, with their
    /// constraints, and so does one whose index the new layout has no
    /// meaning for (a grid's `e5[2,3]` in a pattern in one direction).
    pub fn replace_nested_pattern(
        &mut self,
        id: PatternId,
        seed: Vec<EntityId>,
        patterns: Vec<PatternId>,
        count: Expr,
        layout: SketchPatternLayout,
        skip: Vec<InstanceIndex>,
    ) -> Result<Removal, SketchError> {
        let mut next = self.clone();
        let pattern = SketchPattern {
            id,
            seed,
            count,
            layout,
            patterns,
            skip,
        };
        next.check_pattern(&pattern, Some(id))?;
        let Some(slot) = next.patterns.iter_mut().find(|p| p.id == id) else {
            return invalid(format!("the sketch has no pattern {id}"));
        };
        *slot = pattern;
        let mut removal = Removal::default();
        next.settle(&mut removal, &[])?;
        let misfit: Vec<EntityId> = next
            .entities
            .iter()
            .filter(|e| match &e.geometry {
                Geometry::Image { pattern, index, .. } => {
                    *pattern == id && next.index_fault(*pattern, *index).is_some()
                }
                _ => false,
            })
            .map(|e| e.id)
            .collect();
        for image in misfit {
            removal.constraints.extend(next.remove_one(image)?);
            removal.images.push(image);
        }
        next.settle(&mut removal, &[])?;
        *self = next;
        Ok(removal)
    }

    /// Why an instance index has no meaning in a pattern: a grid index in
    /// a pattern in one direction, or the other way round.
    fn index_fault(&self, pattern: PatternId, index: InstanceIndex) -> Option<String> {
        let Some(found) = self.pattern(pattern) else {
            return Some(format!("the sketch has no pattern {pattern}"));
        };
        match (index, found.layout.is_grid()) {
            (InstanceIndex::Last, _)
            | (InstanceIndex::At(_), false)
            | (InstanceIndex::Grid(_), true) => None,
            (InstanceIndex::At(k), true) => Some(format!(
                "{pattern} is a grid: an instance of it has two numbers, as [{k},0]"
            )),
            (InstanceIndex::Grid([i, j]), false) => Some(format!(
                "{pattern} goes one way: an instance of it has one number, not [{i},{j}]"
            )),
        }
    }

    /// The innermost pattern whose own seed has the point `of`, if there
    /// is one: where the levels of an image point of it start.
    pub fn image_owner(&self, of: EntityId) -> Option<PatternId> {
        self.owner_of(of, "").ok()
    }

    /// The innermost pattern whose own seed has the point `of`: where the
    /// levels of an image point of it start. Several patterns may have it
    /// only when each holds the next (a point shared by a seed curve and
    /// the curve of the pattern around it).
    fn owner_of(&self, of: EntityId, shown: &str) -> Result<PatternId, SketchError> {
        let owners: Vec<PatternId> = self
            .patterns
            .iter()
            .filter(|p| self.seed_points(p).contains(&of))
            .map(|p| p.id)
            .collect();
        match owners.as_slice() {
            [one] => Ok(*one),
            [] => invalid(format!(
                "{shown}: {of} is not a point of the seed of any pattern"
            )),
            many => {
                // The one inside all the others, when they are one chain.
                let inner = many.iter().copied().find(|candidate| {
                    let levels = self.pattern_levels(*candidate);
                    many.iter().all(|p| levels.contains(p))
                });
                match inner {
                    Some(inner) => Ok(inner),
                    None => invalid(format!(
                        "{shown}: {of} is a point of the seeds of {}; an image point needs one pattern",
                        many.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" and ")
                    )),
                }
            }
        }
    }

    /// The image point of seed point `of` in instance `index`: the one
    /// there is, or a new one. `of` has to be a point of the seed of
    /// exactly one pattern (or of patterns that hold one another: the
    /// innermost one's instance).
    pub fn image_point(
        &mut self,
        of: EntityId,
        index: InstanceIndex,
    ) -> Result<EntityId, SketchError> {
        self.image_path(of, &[index])
    }

    /// The image point of seed point `of` with an instance on each level
    /// of its tree of patterns, the innermost first (`e8[last][2]`): one
    /// image point per level, each the image of the one before in the
    /// pattern around it, made when not there. Nothing changes when it is
    /// refused.
    pub fn image_path(
        &mut self,
        of: EntityId,
        levels: &[InstanceIndex],
    ) -> Result<EntityId, SketchError> {
        let shown = format!(
            "{of}{}",
            levels.iter().map(|i| format!("[{i}]")).collect::<String>()
        );
        match &self.entity(of)?.geometry {
            Geometry::Point { .. } => {}
            other => {
                return invalid(format!(
                    "{shown}: {of} is a {}; an image point is the image of a point of a pattern's seed",
                    other.type_name()
                ));
            }
        }
        if levels.is_empty() || levels.len() > MAX_PATTERN_DEPTH {
            return invalid(format!(
                "{shown}: an image point has an instance for each of 1 to {MAX_PATTERN_DEPTH} levels of patterns"
            ));
        }
        let owner = self.owner_of(of, &shown)?;
        let patterns = self.pattern_levels(owner);
        if levels.len() > patterns.len() {
            return invalid(format!(
                "{shown}: {of} is repeated by {} level{} of patterns ({}), not {}",
                patterns.len(),
                if patterns.len() == 1 { "" } else { "s" },
                patterns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" in "),
                levels.len()
            ));
        }
        for (index, pattern) in levels.iter().zip(&patterns) {
            if let Some(fault) = self.index_fault(*pattern, *index) {
                return invalid(format!("{shown}: {fault}"));
            }
        }
        let (entities, next_id) = (self.entities.len(), self.next_entity_id);
        let mut point = of;
        for (index, pattern) in levels.iter().zip(&patterns) {
            let wanted = Geometry::Image {
                of: point,
                pattern: *pattern,
                index: *index,
            };
            point = match self.entities.iter().find(|e| e.geometry == wanted) {
                Some(there) => there.id,
                None => match self.push(wanted, false) {
                    Ok(id) => id,
                    Err(error) => {
                        self.entities.truncate(entities);
                        self.next_entity_id = next_id;
                        return Err(error);
                    }
                },
            };
        }
        Ok(point)
    }
    /// Checks the rules every sketch built through this API satisfies, for
    /// sketches that come from a file.
    pub fn validate(&self) -> Result<(), SketchError> {
        if self.entities.len() > MAX_SKETCH_ENTITIES {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_ENTITIES} entities"
            ));
        }
        if self.constraints.len() > MAX_SKETCH_CONSTRAINTS {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_CONSTRAINTS} constraints"
            ));
        }
        if self.patterns.len() > MAX_SKETCH_PATTERNS {
            return invalid(format!(
                "a sketch holds at most {MAX_SKETCH_PATTERNS} patterns"
            ));
        }
        let mut ids = BTreeSet::new();
        for entity in &self.entities {
            let id = entity.id;
            if id.0 > MAX_ENTITY_ID {
                return invalid(format!("entity id {} is beyond {MAX_ENTITY_ID}", id.0));
            }
            if id.0 == 0 || id.0 >= self.next_entity_id {
                return invalid(format!(
                    "entity id {} is not below the next free id {}",
                    id.0, self.next_entity_id
                ));
            }
            if !ids.insert(id) {
                return invalid(format!("two entities have the id {id}"));
            }
        }
        // Where each entity is in the list, and the points each pattern
        // repeats, for the image points.
        let order: std::collections::BTreeMap<EntityId, usize> = self
            .entities
            .iter()
            .enumerate()
            .map(|(at, e)| (e.id, at))
            .collect();
        let mut tree: std::collections::BTreeMap<PatternId, BTreeSet<EntityId>> =
            std::collections::BTreeMap::new();
        for entity in &self.entities {
            let id = entity.id;
            let mut spline = false;
            match &entity.geometry {
                Geometry::Point { x, y } => {
                    if !(x.is_finite() && y.is_finite()) {
                        return invalid(format!("the coordinates of {id} are not finite"));
                    }
                }
                Geometry::Circle { radius, .. } => {
                    if !(radius.is_finite() && *radius > 0.0) {
                        return invalid(format!("the radius of {id} is not a positive number"));
                    }
                }
                Geometry::Line { .. } | Geometry::Arc { .. } => {}
                Geometry::Spline { degree, control } => {
                    spline = true;
                    if let Some(fault) = Geometry::spline_fault(*degree, control) {
                        return invalid(format!("{id}: {fault}"));
                    }
                }
                Geometry::Image { of, pattern, index } => {
                    let Some(found) = self.pattern(*pattern) else {
                        return invalid(format!("{id}: the sketch has no pattern {pattern}"));
                    };
                    // A point of the pattern's own seed, or of a pattern in
                    // it (an image point of one of those too), drawn
                    // before this one: images are placed in that order.
                    let of_kind = &self.entity(*of)?.geometry;
                    let repeated =
                        matches!(of_kind, Geometry::Point { .. } | Geometry::Image { .. })
                            && tree
                                .entry(*pattern)
                                .or_insert_with(|| self.tree_points(found))
                                .contains(of);
                    if !repeated {
                        return invalid(format!(
                            "{id}: {of} is not a point of the seed of {pattern}"
                        ));
                    }
                    if order
                        .get(of)
                        .is_none_or(|at| order.get(&id).is_none_or(|me| at >= me))
                    {
                        return invalid(format!(
                            "{id}: it is the image of {of}, which comes after it; an image point comes after the point it is the image of"
                        ));
                    }
                    if let Some(fault) = self.index_fault(*pattern, *index) {
                        return invalid(format!("{id}: {fault}"));
                    }
                }
            }
            let points = entity.geometry.points();
            for point in &points {
                self.point_entity(*point)?;
            }
            if let Some(fault) = self.image_fault(&entity.geometry) {
                return invalid(format!("{id}: {fault}"));
            }
            let distinct: BTreeSet<_> = points.iter().collect();
            if !spline && distinct.len() != points.len() {
                return invalid(format!("{id} uses the same point twice"));
            }
        }
        let mut ids = BTreeSet::new();
        for constraint in &self.constraints {
            let id = constraint.id;
            if id.0 == 0 || id.0 >= self.next_constraint_id {
                return invalid(format!(
                    "constraint id {} is not below the next free id {}",
                    id.0, self.next_constraint_id
                ));
            }
            if !ids.insert(id) {
                return invalid(format!("two constraints have the id {id}"));
            }
            self.check_constraint(&constraint.kind)
                .map_err(|e| SketchError::Invalid(format!("{id}: {e}")))?;
        }
        let mut ids = BTreeSet::new();
        for pattern in &self.patterns {
            let id = pattern.id;
            if id.0 == 0 || id.0 >= self.next_pattern_id {
                return invalid(format!(
                    "pattern id {} is not below the next free id {}",
                    id.0, self.next_pattern_id
                ));
            }
            if !ids.insert(id) {
                return invalid(format!("two patterns have the id {id}"));
            }
            self.check_pattern(pattern, Some(id))?;
        }
        // The places of image points: one per image point at most, in
        // ascending order of id, with finite numbers.
        let images = self.image_ids();
        let mut last = None;
        for place in &self.image_places {
            let id = place.entity;
            if last.is_some_and(|last| last >= id) {
                return invalid(format!(
                    "the image places are not in ascending order of entity, one each ({id})"
                ));
            }
            last = Some(id);
            if !images.contains(&id) {
                return invalid(format!(
                    "an image place is of {id}, which is no image point"
                ));
            }
            if !place.is_finite() {
                return invalid(format!("the image place of {id} is not finite"));
            }
        }
        Ok(())
    }

    fn image_ids(&self) -> BTreeSet<EntityId> {
        self.entities
            .iter()
            .filter(|e| matches!(e.geometry, Geometry::Image { .. }))
            .map(|e| e.id)
            .collect()
    }

    /// Puts the places of image points in the form [`Sketch::validate`]
    /// asks for, for a sketch read from a file: drops the places of
    /// entities that are not image points, those that are not finite and
    /// all but the first for an image point, and sorts the rest by entity.
    /// A place is where the next solve starts carrying an image from, not
    /// part of the design: without one the image is not carried, as before
    /// places were stored. Returns how many were dropped. `n log n` in the
    /// number of places and entities.
    pub fn tidy_image_places(&mut self) -> usize {
        let images = self.image_ids();
        let before = self.image_places.len();
        let mut seen = BTreeSet::new();
        self.image_places
            .retain(|p| images.contains(&p.entity) && p.is_finite() && seen.insert(p.entity));
        self.image_places.sort_by_key(|p| p.entity);
        before - self.image_places.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit square e1..e4 (points) with sides e5..e8, and a circle e10 on
    /// its own centre point e9.
    fn square() -> Sketch {
        let mut s = Sketch::default();
        let p: Vec<EntityId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .into_iter()
            .map(|(x, y)| s.add_point(x, y).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4], false).unwrap();
        }
        let c = s.add_point(0.5, 0.5).unwrap();
        s.add_circle(c, 0.25, false).unwrap();
        s
    }

    fn e(n: u64) -> EntityId {
        EntityId(n)
    }

    fn kind(
        type_name: &str,
        entities: &[u64],
        value: Option<&str>,
    ) -> Result<ConstraintKind, String> {
        let ids: Vec<EntityId> = entities.iter().map(|n| e(*n)).collect();
        ConstraintKind::from_parts(
            type_name,
            &ids,
            value.map(Expr::new),
            DistanceDirection::Direct,
        )
    }

    #[test]
    fn entities_get_ids_in_order_and_are_built_on_points() {
        let mut s = square();
        assert_eq!(s.entities.len(), 10);
        assert_eq!(
            s.entity(e(5)).unwrap().geometry,
            Geometry::Line {
                start: e(1),
                end: e(2)
            }
        );
        assert_eq!(s.point(e(3)).unwrap(), (1.0, 1.0));
        assert_eq!(e(7).to_string(), "e7");
        assert_eq!(ConstraintId(2).to_string(), "c2");
        // Built on something that is not a point, or does not exist.
        assert!(matches!(
            s.add_line(e(1), e(5), false),
            Err(SketchError::Invalid(_))
        ));
        assert!(matches!(
            s.add_line(e(1), e(99), false),
            Err(SketchError::UnknownEntity(_))
        ));
        assert!(s.add_line(e(1), e(1), false).is_err());
        assert!(s.add_circle(e(10), 1.0, false).is_err());
        assert!(s.add_arc(e(1), e(2), e(2), false).is_err());
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(s.add_circle(e(9), bad, false).is_err(), "{bad}");
        }
        assert!(s.add_point(f64::NAN, 0.0).is_err());
        assert!(s.add_point(0.0, f64::NEG_INFINITY).is_err());
        assert_eq!(s.entities.len(), 10, "failed adds add nothing");
        assert_eq!(s.next_entity_id, 11, "and use up no ids");
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn every_constraint_type_accepts_its_entities_and_refuses_others() {
        let mut s = square();
        let arc_start = s.add_point(2.0, 0.0).unwrap();
        let arc_end = s.add_point(0.0, 2.0).unwrap();
        let arc = s.add_arc(e(1), arc_start, arc_end, false).unwrap().0;
        // (type, entities that fit, entities that do not)
        let cases: [(&str, &[u64], &[u64]); 22] = [
            ("coincident", &[1, 2], &[1, 5]),
            ("pointOn", &[1, 6], &[5, 6]),
            ("pointOn", &[1, 10], &[10, 1]),
            ("horizontal", &[5], &[10]),
            ("horizontal", &[1, 2], &[1, 5]),
            ("vertical", &[6], &[1]),
            ("parallel", &[5, 7], &[5, 10]),
            ("perpendicular", &[5, 6], &[1, 2]),
            ("tangent", &[5, 10], &[5, 6]),
            ("tangent", &[10, arc], &[10, 1]),
            ("equal", &[5, 6], &[5, 10]),
            ("equal", &[10, arc], &[1, 2]),
            ("concentric", &[10, arc], &[10, 5]),
            ("symmetric", &[1, 2, 6], &[1, 2, 3]),
            ("midpoint", &[9, 5], &[5, 9]),
            ("fixed", &[1], &[5]),
            ("distance", &[5], &[10]),
            ("distance", &[1, 3], &[5, 6]),
            ("distance", &[1, 6], &[10, 1]),
            ("angle", &[5, 6], &[5, 10]),
            ("radius", &[arc], &[5]),
            ("diameter", &[10], &[1]),
        ];
        for (type_name, good, bad) in cases {
            let value = ConstraintKind::has_value(type_name).then_some("1");
            let fits = kind(type_name, good, value).unwrap();
            assert_eq!(fits.type_name(), type_name);
            assert_eq!(
                fits.entities(),
                good.iter().map(|n| e(*n)).collect::<Vec<_>>()
            );
            assert_eq!(s.check_constraint(&fits), Ok(()), "{type_name} {good:?}");
            let wrong = kind(type_name, bad, value).unwrap();
            assert!(
                matches!(s.check_constraint(&wrong), Err(SketchError::Invalid(_))),
                "{type_name} {bad:?}"
            );
        }
        let covered: BTreeSet<&str> = cases.iter().map(|c| c.0).collect();
        assert_eq!(
            covered.len(),
            ConstraintKind::TYPES.len(),
            "every type has a case"
        );
        // The same entity twice, and an entity that is not there.
        assert!(
            s.check_constraint(&kind("coincident", &[1, 1], None).unwrap())
                .is_err()
        );
        assert!(matches!(
            s.check_constraint(&kind("parallel", &[5, 77], None).unwrap()),
            Err(SketchError::UnknownEntity(EntityId(77)))
        ));
        // A horizontal distance to a line has no meaning.
        let sideways = ConstraintKind::Distance {
            a: e(1),
            b: Some(e(6)),
            direction: DistanceDirection::Horizontal,
            value: Expr::new("1"),
        };
        assert!(s.check_constraint(&sideways).is_err());
    }

    #[test]
    fn a_spline_is_built_on_points_and_refused_with_a_reason() {
        let mut s = square();
        let before = s.clone();
        // (degree, control points, what the refusal says)
        let bad: [(u32, &[u64], &str); 9] = [
            (0, &[1, 2, 3], "degree"),
            (MAX_SPLINE_DEGREE + 1, &[1, 2, 3, 4, 9], "degree"),
            (u32::MAX, &[1, 2, 3], "degree"),
            (3, &[1, 2, 3], "at least 4"),
            (2, &[], "at least 3"),
            (2, &[1, 2, 2, 3], "twice"),
            (2, &[1, 2, 3, 2], "twice"),
            (1, &[1, 1], "twice"),
            (2, &[1, 2, 5], "not a point"),
        ];
        for (degree, control, what) in bad {
            let ids: Vec<EntityId> = control.iter().map(|n| e(*n)).collect();
            let error = s.add_spline(degree, ids, false).unwrap_err().to_string();
            assert!(error.contains(what), "{degree} {control:?}: {error}");
        }
        assert!(matches!(
            s.add_spline(2, vec![e(1), e(2), e(77)], false),
            Err(SketchError::UnknownEntity(EntityId(77)))
        ));
        let many: Vec<EntityId> = (0..=MAX_SPLINE_POINTS as u64).map(EntityId).collect();
        assert!(s.add_spline(3, many, false).is_err());
        assert_eq!(s, before, "failed adds add nothing and use up no ids");

        let open = s
            .add_spline(3, vec![e(1), e(2), e(3), e(4)], false)
            .unwrap();
        let closed = s.add_spline(2, vec![e(1), e(9), e(3), e(1)], true).unwrap();
        assert_eq!((open, closed), (e(11), e(12)));
        let geometry = &s.entity(open).unwrap().geometry;
        assert_eq!(geometry.type_name(), "spline");
        assert_eq!(geometry.points(), [e(1), e(2), e(3), e(4)]);
        assert_eq!(geometry.ends(), Some((e(1), e(4))));
        assert_eq!(
            s.entity(closed).unwrap().geometry.ends(),
            Some((e(1), e(1)))
        );
        assert_eq!(s.validate(), Ok(()));
        // Its points cannot go before it does.
        assert_eq!(
            s.remove_entity(e(9)),
            Err(SketchError::InUse {
                entity: e(9),
                by: vec![e(10), closed]
            })
        );
        assert_eq!(s.remove_entity(closed), Ok(vec![]));

        // The JSON form.
        let json = serde_json::to_value(&s.entity(open).unwrap().geometry).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "spline", "degree": 3, "control": [1, 2, 3, 4]})
        );
        assert_eq!(
            &serde_json::from_value::<Geometry>(json).unwrap(),
            &s.entity(open).unwrap().geometry
        );

        // A file may say anything.
        type Damage = fn(&mut Geometry);
        let broken: [Damage; 4] = [
            |g| {
                *g = Geometry::Spline {
                    degree: 0,
                    control: vec![EntityId(1), EntityId(2)],
                }
            },
            |g| {
                *g = Geometry::Spline {
                    degree: 3,
                    control: vec![EntityId(1), EntityId(2)],
                }
            },
            |g| {
                *g = Geometry::Spline {
                    degree: 1,
                    control: vec![EntityId(1), EntityId(40)],
                }
            },
            |g| {
                *g = Geometry::Spline {
                    degree: 1,
                    control: vec![EntityId(1), EntityId(2), EntityId(2)],
                }
            },
        ];
        for (i, damage) in broken.into_iter().enumerate() {
            let mut damaged = s.clone();
            damage(&mut damaged.entity_mut(open).unwrap().geometry);
            assert!(damaged.validate().is_err(), "damage {i}");
        }
    }

    #[test]
    fn the_knots_of_a_spline_are_clamped_and_uniform() {
        assert_eq!(
            clamped_uniform_knots(3, 4),
            [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(
            clamped_uniform_knots(2, 5),
            [0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(clamped_uniform_knots(1, 3), [0.0, 0.0, 0.5, 1.0, 1.0]);
        for (degree, count) in [(0, 5), (3, 3), (MAX_SPLINE_DEGREE + 1, 20), (u32::MAX, 9)] {
            assert!(clamped_uniform_knots(degree, count).is_empty());
        }
        assert!(clamped_uniform_knots(3, MAX_SPLINE_POINTS + 1).is_empty());
        for degree in 1..=MAX_SPLINE_DEGREE {
            for count in (degree as usize + 1)..40 {
                let knots = clamped_uniform_knots(degree, count);
                assert_eq!(knots.len(), count + degree as usize + 1);
                assert!(knots.windows(2).all(|w| w[0] <= w[1]));
            }
        }
    }

    #[test]
    fn a_spline_is_tangent_to_a_curve_it_shares_an_end_with() {
        let mut s = square();
        // From corner e2 over two new points to corner e3: it shares e2
        // with the sides e5 and e6.
        let a = s.add_point(2.0, 0.2).unwrap();
        let b = s.add_point(2.0, 0.8).unwrap();
        let spline = s.add_spline(3, vec![e(2), a, b, e(3)], false).unwrap();
        assert_eq!(s.tangent_at(spline, e(5)), Some((e(2), e(2))));
        assert_eq!(s.tangent_at(e(7), spline), Some((e(3), e(3))));
        assert_eq!(s.tangent_at(spline, e(8)), None);
        assert_eq!(s.tangent_at(spline, e(10)), None, "a circle has no ends");
        let tangent = |x: EntityId, y: EntityId| ConstraintKind::Tangent { a: x, b: y };
        assert!(s.add_constraint(tangent(spline, e(5))).is_ok());
        assert!(s.add_constraint(tangent(e(7), spline)).is_ok());
        let error = s.add_constraint(tangent(spline, e(8))).unwrap_err();
        assert!(error.to_string().contains("share none"), "{error}");
        // Ends tied by a coincident constraint are shared too.
        let far = s.add_point(0.0, 1.0).unwrap();
        let other = s.add_spline(1, vec![far, a], true).unwrap();
        assert_eq!(s.tangent_at(other, e(8)), None);
        s.add_constraint(ConstraintKind::Coincident { a: far, b: e(4) })
            .unwrap();
        assert_eq!(s.tangent_at(other, e(8)), Some((far, e(4))));
        assert!(s.add_constraint(tangent(other, e(8))).is_ok());
        // What a spline cannot be given to says so.
        for (type_name, entities) in [
            ("equal", vec![spline, other]),
            ("parallel", vec![spline, e(5)]),
            ("fixed", vec![spline]),
        ] {
            let kind =
                ConstraintKind::from_parts(type_name, &entities, None, DistanceDirection::Direct)
                    .unwrap();
            let error = s.check_constraint(&kind).unwrap_err().to_string();
            assert!(
                error.contains("not supported for splines yet"),
                "{type_name}: {error}"
            );
        }
        // A point on a spline is taken (since 2026-10-14).
        let on =
            ConstraintKind::from_parts("pointOn", &[e(1), spline], None, DistanceDirection::Direct)
                .unwrap();
        assert_eq!(s.check_constraint(&on), Ok(()));
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn a_constraint_is_built_from_parts_or_refused_with_a_reason() {
        let message = |r: Result<ConstraintKind, String>| r.unwrap_err();
        assert!(message(kind("paralel", &[1, 2], None)).contains("unknown constraint type"));
        assert!(message(kind("parallel", &[1], None)).contains("but 1 were given"));
        assert!(message(kind("symmetric", &[1, 2], None)).contains("but 2 were given"));
        assert!(message(kind("distance", &[1, 2], None)).contains("needs a `value`"));
        assert!(message(kind("parallel", &[5, 6], Some("3"))).contains("takes no `value`"));
        assert!(
            ConstraintKind::from_parts(
                "radius",
                &[e(10)],
                Some(Expr::new("1")),
                DistanceDirection::Vertical
            )
            .unwrap_err()
            .contains("takes no `direction`")
        );
        assert!(kind("fixed", &[], None).is_err());
        let d = kind("distance", &[5], Some("width")).unwrap();
        assert_eq!(d.value().unwrap().source(), "width");
        assert_eq!(kind("fixed", &[1], None).unwrap().value(), None);
    }

    #[test]
    fn deleting_an_entity_takes_its_constraints_along() {
        let mut s = square();
        let on_side = s
            .add_constraint(kind("horizontal", &[5], None).unwrap())
            .unwrap();
        let on_point = s
            .add_constraint(kind("fixed", &[1], None).unwrap())
            .unwrap();
        let other = s
            .add_constraint(kind("vertical", &[6], None).unwrap())
            .unwrap();
        assert_eq!(
            (on_side, on_point, other),
            (ConstraintId(1), ConstraintId(2), ConstraintId(3))
        );
        // A point two lines are built on stays.
        assert_eq!(
            s.remove_entity(e(1)),
            Err(SketchError::InUse {
                entity: e(1),
                by: vec![e(5), e(8)]
            })
        );
        assert_eq!(s.remove_entity(e(5)), Ok(vec![on_side]));
        assert_eq!(s.remove_entity(e(8)), Ok(vec![]));
        assert_eq!(s.remove_entity(e(1)), Ok(vec![on_point]));
        assert!(matches!(
            s.remove_entity(e(1)),
            Err(SketchError::UnknownEntity(_))
        ));
        assert_eq!(
            s.constraints.iter().map(|c| c.id).collect::<Vec<_>>(),
            [other]
        );
        s.remove_constraint(other).unwrap();
        assert!(matches!(
            s.remove_constraint(other),
            Err(SketchError::UnknownConstraint(_))
        ));
        // Ids are not reused.
        assert_eq!(s.add_point(0.0, 0.0), Ok(e(11)));
        assert_eq!(
            s.add_constraint(kind("fixed", &[11], None).unwrap()),
            Ok(ConstraintId(4))
        );
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn validation_rejects_sketches_that_break_the_rules() {
        let mut base = square();
        base.add_constraint(kind("distance", &[5], Some("1")).unwrap())
            .unwrap();
        assert_eq!(base.validate(), Ok(()));
        type Damage = fn(&mut Sketch);
        let broken: [(&str, Damage); 10] = [
            ("duplicate entity id", |s| s.entities[1].id = EntityId(1)),
            ("entity id zero", |s| s.entities[9].id = EntityId(0)),
            ("entity id not below next", |s| s.next_entity_id = 10),
            ("NaN coordinate", |s| {
                s.entities[0].geometry = Geometry::Point {
                    x: f64::NAN,
                    y: 0.0,
                }
            }),
            ("negative radius", |s| {
                s.entities[9].geometry = Geometry::Circle {
                    center: EntityId(9),
                    radius: -1.0,
                }
            }),
            ("line on a missing point", |s| {
                s.entities[4].geometry = Geometry::Line {
                    start: EntityId(1),
                    end: EntityId(40),
                }
            }),
            ("line on a line", |s| {
                s.entities[5].geometry = Geometry::Line {
                    start: EntityId(1),
                    end: EntityId(5),
                }
            }),
            ("line from a point to itself", |s| {
                s.entities[4].geometry = Geometry::Line {
                    start: EntityId(1),
                    end: EntityId(1),
                }
            }),
            ("constraint id not below next", |s| s.next_constraint_id = 1),
            ("constraint on the wrong kind", |s| {
                s.constraints[0].kind = ConstraintKind::Fixed { point: EntityId(5) }
            }),
        ];
        for (what, damage) in broken {
            let mut s = base.clone();
            damage(&mut s);
            assert!(s.validate().is_err(), "{what}");
        }
    }

    #[test]
    fn the_json_form_is_stable_and_round_trips() {
        let mut s = square();
        s.entities[8].construction = true;
        s.add_constraint(kind("horizontal", &[5], None).unwrap())
            .unwrap();
        s.add_constraint(ConstraintKind::Distance {
            a: e(1),
            b: Some(e(2)),
            direction: DistanceDirection::Horizontal,
            value: Expr::new("width"),
        })
        .unwrap();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json["plane"],
            serde_json::json!({"type": "world", "plane": "xy", "offset": "0"})
        );
        assert_eq!(
            json["entities"][1],
            serde_json::json!({"id": 2, "construction": false, "geometry": {"type": "point", "x": 1.0, "y": 0.0}})
        );
        assert_eq!(
            json["entities"][4]["geometry"],
            serde_json::json!({"type": "line", "start": 1, "end": 2})
        );
        assert_eq!(
            json["entities"][9]["geometry"],
            serde_json::json!({"type": "circle", "center": 9, "radius": 0.25})
        );
        assert_eq!(
            json["constraints"],
            serde_json::json!([
                {"id": 1, "kind": {"type": "horizontal", "a": 5, "b": null}},
                {"id": 2, "kind": {"type": "distance", "a": 1, "b": 2, "direction": "horizontal", "value": "width"}},
            ])
        );
        assert_eq!(serde_json::from_value::<Sketch>(json).unwrap(), s);
        assert_eq!(
            s.exprs()
                .iter()
                .map(|(n, e)| (n.as_str(), e.source()))
                .collect::<Vec<_>>(),
            [("plane.offset", "0"), ("c2.value", "width")]
        );
        for plane in WorldPlane::ALL {
            assert_eq!(WorldPlane::from_name(plane.name()), Some(plane));
            assert_eq!(
                serde_json::to_value(plane).unwrap(),
                serde_json::json!(plane.name())
            );
        }
    }

    fn row(step: f64) -> SketchPatternLayout {
        SketchPatternLayout::Linear {
            step: [Expr::new(step.to_string()), Expr::new("0")],
            second: None,
        }
    }

    /// A run of tabs (p1, chained e1 to e2) inside a cell (p2, chained e1
    /// to e6): the spacer e5 runs from the end of the inner run, e2[last],
    /// to e6.
    fn nested() -> (Sketch, PatternId, PatternId, EntityId) {
        let mut s = Sketch::default();
        let a = s.add_point(0.0, 0.0).unwrap();
        let b = s.add_point(1.0, 0.0).unwrap();
        let tab = s.add_line(a, b, false).unwrap();
        let chain = |from, to| SketchPatternLayout::Chain { from, to };
        let p1 = s
            .add_pattern(vec![tab], Expr::new("k"), chain(a, b))
            .unwrap();
        let end = s.image_point(b, InstanceIndex::Last).unwrap();
        let far = s.add_point(3.0, 0.0).unwrap();
        let spacer = s.add_line(end, far, false).unwrap();
        let p2 = s
            .add_nested_pattern(
                vec![spacer],
                vec![p1],
                Expr::new("n"),
                chain(a, far),
                vec![],
            )
            .unwrap();
        (s, p1, p2, end)
    }

    #[test]
    fn patterns_nest_as_a_tree_and_their_rules_are_kept() {
        let (mut s, p1, p2, end) = nested();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.pattern_levels(p1), [p1, p2]);
        assert_eq!(s.parent_pattern(p1), Some(p2));
        assert_eq!(s.inner_patterns(p2), BTreeSet::from([p1]));
        // The outer seed repeats the inner one's points and image points.
        let tree = s.tree_points(s.pattern(p2).unwrap());
        assert!(tree.contains(&e(1)) && tree.contains(&e(2)) && tree.contains(&end));
        // An image path: one image point per level, reused.
        let corner = s
            .image_path(e(2), &[InstanceIndex::Last, InstanceIndex::Last])
            .unwrap();
        assert_eq!(
            s.entity(corner).unwrap().geometry,
            Geometry::Image {
                of: end,
                pattern: p2,
                index: InstanceIndex::Last
            }
        );
        assert_eq!(
            s.image_path(e(2), &[InstanceIndex::Last, InstanceIndex::Last]),
            Ok(corner)
        );
        assert_eq!(s.validate(), Ok(()));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["patterns"][1]["patterns"], serde_json::json!([1]));
        assert_eq!(serde_json::from_value::<Sketch>(json).unwrap(), s);
        // Refused paths: too many levels, a level of the wrong kind.
        for (levels, want) in [
            (vec![InstanceIndex::Last; 3], "2 levels"),
            (
                vec![InstanceIndex::At(1), InstanceIndex::Grid([1, 1])],
                "one way",
            ),
            (vec![], "1 to 3"),
        ] {
            let error = s.image_path(e(2), &levels).unwrap_err().to_string();
            assert!(error.contains(want), "{error}");
        }
        // What a tree cannot be.
        let refuse = |s: &Sketch, pattern: SketchPattern, want: &str| {
            let error = s.check_pattern(&pattern, Some(pattern.id)).unwrap_err();
            assert!(error.to_string().contains(want), "{error}");
        };
        let mut outer = s.pattern(p2).unwrap().clone();
        outer.patterns = vec![p2];
        refuse(&s, outer.clone(), "itself");
        outer.patterns = vec![p1, p1];
        refuse(&s, outer.clone(), "twice");
        outer.patterns = vec![PatternId(9)];
        refuse(&s, outer, "no pattern p9");
        let mut inner = s.pattern(p1).unwrap().clone();
        inner.patterns = vec![p2];
        refuse(&s, inner, "cycle");
        // A third pattern may hold p1 only if p2 does not.
        let other = s.add_point(9.0, 9.0).unwrap();
        let error = s
            .add_nested_pattern(vec![other], vec![p1], Expr::new("2"), row(1.0), vec![])
            .unwrap_err();
        assert!(error.to_string().contains("in the seed of p2"), "{error}");
        // Three levels, not four.
        let p3 = s
            .add_nested_pattern(vec![], vec![p2], Expr::new("2"), row(10.0), vec![])
            .unwrap();
        let p4 = s.add_nested_pattern(vec![], vec![p3], Expr::new("2"), row(100.0), vec![]);
        assert!(p4.unwrap_err().to_string().contains("at most 3 levels"));
        assert_eq!(s.pattern_levels(p1), [p1, p2, p3]);
        assert_eq!(s.validate(), Ok(()));
        // A seed built on an image point of a pattern not in it.
        let lone = s.add_point(5.0, 5.0).unwrap();
        let hung = s.add_line(corner, lone, false).unwrap();
        let error = s
            .add_pattern(vec![hung], Expr::new("2"), row(1.0))
            .unwrap_err();
        assert!(error.to_string().contains("not in the seed"), "{error}");

        // Deleting the inner pattern unwraps it: p2 keeps its spacer, but
        // its chain end was not the image, so it stays; the image points
        // of p1 and their images in p2 go (the line on the corner is in
        // the way first).
        let refused = s.remove_pattern(p1).unwrap_err();
        assert!(matches!(refused, SketchError::InUse { .. }), "{refused}");
        s.remove_entity(hung).unwrap();
        // The spacer is built on e2[last]: still in the way.
        assert!(s.remove_pattern(p1).is_err());
    }

    #[test]
    fn an_inner_pattern_deleted_leaves_the_outer_one_whole() {
        let (mut s, p1, p2, end) = nested();
        let corner = s
            .image_path(e(2), &[InstanceIndex::At(1), InstanceIndex::Last])
            .unwrap();
        let spacer = s.pattern(p2).unwrap().seed[0];
        let mut s2 = s.clone();
        let removal = s2.remove_pattern(p2).unwrap();
        assert_eq!(removal.patterns, [p2]);
        assert_eq!(removal.images, [corner]);
        assert!(s2.pattern(p1).is_some() && s2.parent_pattern(p1).is_none());
        assert_eq!(s2.validate(), Ok(()));
        // A pattern of patterns only: its seed is a pattern.
        let mut s3 = s.clone();
        let removal = s3
            .replace_nested_pattern(
                p2,
                vec![],
                vec![p1],
                Expr::new("n"),
                SketchPatternLayout::Chain {
                    from: e(1),
                    to: end,
                },
                vec![],
            )
            .unwrap();
        assert!(removal.images.is_empty());
        assert_eq!(s3.validate(), Ok(()));
        // ... and goes when its last inner pattern does (the old spacer,
        // built on e2[last], deleted first).
        let mut s4 = s3.clone();
        s4.remove_entity(spacer).unwrap();
        let removal = s4.remove_pattern(p1).unwrap();
        assert_eq!(removal.unwrapped, [(p1, p2)]);
        assert_eq!(removal.patterns, [p1, p2]);
        assert_eq!(removal.images, [end, EntityId(7), corner]);
        assert!(s4.patterns.is_empty());
        assert_eq!(s4.validate(), Ok(()));
        // Deleting the spacer of the first sketch leaves p2 without its
        // chain end e5's curve: p2 goes with its image point.
        let removal = s.remove_entity_reporting(spacer).unwrap();
        assert_eq!(removal.patterns, [p2]);
        assert_eq!(removal.images, [corner]);
        assert_eq!(s.parent_pattern(p1), None);
    }

    #[test]
    fn a_hostile_tree_of_patterns_in_a_file_is_refused() {
        let (mut s, p1, p2, _) = nested();
        let corner = s
            .image_path(e(2), &[InstanceIndex::Last, InstanceIndex::Last])
            .unwrap();
        // Four more patterns on lone points, to build trees from.
        let mut extra = Vec::new();
        for i in 0..4 {
            let point = s.add_point(20.0 + f64::from(i), 0.0).unwrap();
            extra.push(
                s.add_pattern(vec![point], Expr::new("2"), row(1.0))
                    .unwrap(),
            );
        }
        assert_eq!(s.validate(), Ok(()));
        let extra: [PatternId; 4] = extra.try_into().unwrap();
        type Breaking = Box<dyn Fn(&mut Sketch)>;
        let broken: [(&str, Breaking); 5] = [
            (
                "cycle",
                Box::new(move |s: &mut Sketch| s.patterns[0].patterns = vec![p2]),
            ),
            (
                "in the seed of",
                Box::new(move |s: &mut Sketch| s.patterns[2].patterns = vec![p1]),
            ),
            (
                "at most 3 levels",
                Box::new(move |s: &mut Sketch| {
                    s.patterns[2].patterns = vec![extra[1]];
                    s.patterns[3].patterns = vec![extra[2]];
                    s.patterns[4].patterns = vec![extra[3]];
                }),
            ),
            (
                "itself",
                Box::new(move |s: &mut Sketch| s.patterns[2].patterns = vec![extra[0]]),
            ),
            (
                "comes after it",
                Box::new(move |s: &mut Sketch| {
                    let at = s.entities.iter().position(|e| e.id == corner).unwrap();
                    let image = s.entities.remove(at);
                    s.entities.insert(0, image);
                }),
            ),
        ];
        for (want, breaking) in broken {
            let mut bad = s.clone();
            breaking(&mut bad);
            let error = bad.validate().unwrap_err().to_string();
            assert!(error.contains(want), "{want}: {error}");
        }
    }

    #[test]
    fn skips_are_a_patterns_own_instances() {
        let mut s = Sketch::default();
        let c = s.add_point(0.0, 0.0).unwrap();
        let disc = s.add_circle(c, 1.0, false).unwrap();
        let p = s
            .add_nested_pattern(
                vec![disc],
                vec![],
                Expr::new("5"),
                row(3.0),
                vec![InstanceIndex::At(3), InstanceIndex::At(7)],
            )
            .unwrap();
        assert_eq!(s.validate(), Ok(()));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json["patterns"][0]["skip"],
            serde_json::json!([{"at": 3}, {"at": 7}])
        );
        let pattern = s.pattern(p).unwrap().clone();
        for (skip, want) in [
            (vec![InstanceIndex::At(0)], "instance 0"),
            (vec![InstanceIndex::Last], "not as last"),
            (
                vec![InstanceIndex::At(4), InstanceIndex::At(2)],
                "ascending",
            ),
            (
                vec![InstanceIndex::At(4), InstanceIndex::At(4)],
                "ascending",
            ),
            (vec![InstanceIndex::Grid([1, 1])], "one way"),
            (
                vec![InstanceIndex::At(1); MAX_PATTERN_INSTANCES + 1],
                "at most",
            ),
        ] {
            let mut bad = pattern.clone();
            bad.skip = skip;
            let error = s.check_pattern(&bad, Some(p)).unwrap_err();
            assert!(error.to_string().contains(want), "{error}");
        }
        // Not in a chained run.
        let a = s.add_point(0.0, 5.0).unwrap();
        let b = s.add_point(1.0, 5.0).unwrap();
        let l = s.add_line(a, b, false).unwrap();
        let error = s
            .add_nested_pattern(
                vec![l],
                vec![],
                Expr::new("3"),
                SketchPatternLayout::Chain { from: a, to: b },
                vec![InstanceIndex::At(1)],
            )
            .unwrap_err();
        assert!(error.to_string().contains("not supported yet"), "{error}");
    }

    #[test]
    fn image_paths_are_read_from_text() {
        let ok = |text: &str| parse_image_path(text).unwrap().unwrap();
        assert_eq!(ok("e8[2]"), (e(8), vec![InstanceIndex::At(2)]));
        assert_eq!(
            ok("e8[last][last]"),
            (e(8), vec![InstanceIndex::Last, InstanceIndex::Last])
        );
        assert_eq!(
            ok("e8[1][2,4]"),
            (
                e(8),
                vec![InstanceIndex::At(1), InstanceIndex::Grid([2, 4])]
            )
        );
        assert!(parse_image_path("e8").is_none());
        for bad in [
            "e8[last][last][last][last]",
            "e8[2][",
            "e8[][1]",
            "e8[1,2,3][0]",
            "e8[1]x",
            "e8[-1]",
            "[1]",
        ] {
            assert!(parse_image_path(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pattern_repeats_seed_entities_and_its_rules_are_kept() {
        let mut s = square();
        // The circle e10 in a row; its centre e9 comes along.
        let p1 = s
            .add_pattern(vec![e(10)], Expr::new("n"), row(2.0))
            .unwrap();
        assert_eq!((p1, p1.to_string()), (PatternId(1), "p1".to_string()));
        let pattern = s.pattern(p1).unwrap().clone();
        assert_eq!(
            s.seed_points(&pattern).into_iter().collect::<Vec<_>>(),
            [e(9)]
        );
        assert_eq!(
            s.exprs()
                .iter()
                .map(|(n, x)| (n.as_str(), x.source()))
                .collect::<Vec<_>>(),
            [
                ("plane.offset", "0"),
                ("p1.count", "n"),
                ("p1.step.0", "2"),
                ("p1.step.1", "0")
            ]
        );
        // What a pattern cannot be.
        let refused = |s: &mut Sketch, seed: Vec<u64>, layout: SketchPatternLayout, want: &str| {
            let before = s.clone();
            let error = s
                .add_pattern(seed.into_iter().map(e).collect(), Expr::new("3"), layout)
                .unwrap_err()
                .to_string();
            assert!(error.contains(want), "{error}");
            assert_eq!(*s, before, "nothing changes");
        };
        refused(&mut s, vec![], row(1.0), "at least one");
        refused(&mut s, vec![5, 5], row(1.0), "twice");
        refused(&mut s, vec![10], row(1.0), "seed of p1");
        refused(&mut s, vec![77], row(1.0), "e77");
        refused(
            &mut s,
            vec![5, 6],
            SketchPatternLayout::Chain {
                from: e(1),
                to: e(1),
            },
            "twice",
        );
        refused(
            &mut s,
            vec![5, 6],
            SketchPatternLayout::Chain {
                from: e(1),
                to: e(4),
            },
            "not a point of the seed",
        );
        refused(
            &mut s,
            vec![5],
            SketchPatternLayout::Circular {
                center: CenterRef::Point { point: e(5) },
                angle: Expr::new("30 deg"),
                chain: None,
            },
            "not a point",
        );
        // A chained run of the sides e5 and e6, from e1 to e3.
        let p2 = s
            .add_pattern(
                vec![e(5), e(6)],
                Expr::new("4"),
                SketchPatternLayout::Chain {
                    from: e(1),
                    to: e(3),
                },
            )
            .unwrap();
        assert_eq!(s.validate(), Ok(()));

        // Image points: one per seed point and instance, made once.
        let last = s.image_point(e(3), InstanceIndex::Last).unwrap();
        assert_eq!(s.image_point(e(3), InstanceIndex::Last), Ok(last));
        assert!(s.is_image(last));
        let third = s.image_point(e(9), InstanceIndex::At(3)).unwrap();
        assert_ne!(third, last);
        for (of, index, want) in [
            (e(4), InstanceIndex::At(1), "not a point of the seed"),
            (e(5), InstanceIndex::At(1), "line"),
            (e(9), InstanceIndex::Grid([1, 1]), "one number"),
            (e(77), InstanceIndex::Last, "e77"),
        ] {
            let error = s.image_point(of, index).unwrap_err().to_string();
            assert!(error.contains(want), "{of}[{index}]: {error}");
        }
        // A line may end on an image point; nothing else may be built on
        // one, and no seed may hold one.
        let far = s.add_point(9.0, 9.0).unwrap();
        let tail = s.add_line(last, far, false).unwrap();
        assert!(s.add_circle(last, 1.0, false).is_err());
        let error = s
            .add_pattern(vec![tail], Expr::new("2"), row(1.0))
            .unwrap_err();
        assert!(error.to_string().contains("image point"), "{error}");
        let error = s
            .add_pattern(vec![last], Expr::new("2"), row(1.0))
            .unwrap_err();
        assert!(error.to_string().contains("image point"), "{error}");
        // Constraints on images: with other entities, yes; between images
        // of one pattern only, no.
        let tied = ConstraintKind::Coincident { a: far, b: third };
        assert!(s.add_constraint(tied).is_ok());
        let other = s.image_point(e(9), InstanceIndex::At(1)).unwrap();
        let between = ConstraintKind::Coincident { a: other, b: third };
        let error = s.add_constraint(between).unwrap_err().to_string();
        assert!(error.contains("not supported yet"), "{error}");
        assert_eq!(s.validate(), Ok(()));

        // Deleting: the line on the image keeps it; the seed curve leaves
        // its pattern; a pattern without its chain end goes with its images.
        assert!(matches!(
            s.remove_entity_reporting(last),
            Err(SketchError::InUse { .. })
        ));
        s.remove_entity(tail).unwrap();
        let removal = s.remove_entity_reporting(e(6)).unwrap();
        // e3 (the chain end) was only in e6: p2 is gone, and e3[last].
        assert_eq!(removal.patterns, [p2]);
        assert_eq!(removal.images, [last]);
        assert!(s.pattern(p2).is_none());
        // Removing p1 takes its two images and the constraint on one.
        let removal = s.remove_pattern(p1).unwrap();
        assert_eq!(removal.images, [third, other]);
        assert_eq!(removal.constraints.len(), 1);
        assert!(s.patterns.is_empty());
        assert_eq!(s.validate(), Ok(()));
        assert!(s.remove_pattern(p1).is_err());
        // Ids are not reused.
        assert_eq!(
            s.add_pattern(vec![e(10)], Expr::new("2"), row(1.0)),
            Ok(PatternId(3))
        );
    }

    #[test]
    fn a_sketch_with_patterns_round_trips_and_a_broken_one_is_refused() {
        let mut s = square();
        let p = s
            .add_pattern(
                vec![e(10)],
                Expr::new("3"),
                SketchPatternLayout::Circular {
                    center: CenterRef::At {
                        at: [Expr::new("0"), Expr::new("1 mm")],
                    },
                    angle: Expr::new("120 deg"),
                    chain: None,
                },
            )
            .unwrap();
        s.image_point(e(9), InstanceIndex::Last).unwrap();
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["next_pattern_id"], 2);
        assert_eq!(
            json["entities"][10]["geometry"],
            serde_json::json!({"type": "image", "of": 9, "pattern": 1, "index": "last"})
        );
        assert_eq!(serde_json::from_value::<Sketch>(json).unwrap(), s);
        // Without patterns the JSON is what it was.
        let plain = serde_json::to_value(square()).unwrap();
        assert!(plain.get("patterns").is_none() && plain.get("next_pattern_id").is_none());

        type Damage = fn(&mut Sketch);
        let broken: [(&str, Damage); 5] = [
            ("pattern id not below next", |s| s.next_pattern_id = 1),
            ("image of no pattern", |s| {
                s.entities[10].geometry = Geometry::Image {
                    of: EntityId(9),
                    pattern: PatternId(7),
                    index: InstanceIndex::Last,
                }
            }),
            ("image of a point not in the seed", |s| {
                s.entities[10].geometry = Geometry::Image {
                    of: EntityId(1),
                    pattern: PatternId(1),
                    index: InstanceIndex::Last,
                }
            }),
            ("seed missing", |s| s.patterns[0].seed = vec![EntityId(99)]),
            ("entity id beyond 32 bits", |s| {
                s.entities[0].id = EntityId(MAX_ENTITY_ID + 1);
                s.next_entity_id = MAX_ENTITY_ID + 2;
            }),
        ];
        for (what, damage) in broken {
            let mut damaged = s.clone();
            damage(&mut damaged);
            assert!(damaged.validate().is_err(), "{what}");
        }
        assert_eq!(s.pattern(p).map(|p| p.layout.type_name()), Some("circular"));
    }
}
