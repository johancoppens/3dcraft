//! Sketches: 2D entities on a plane, tied together by constraints.
//!
//! Pure data, like the rest of this crate. A sketch stores what the user
//! drew and constrained, in the order it was done, plus the coordinates of
//! the last solution: the numbers in [`Geometry::Point`] and
//! [`Geometry::Circle`] are where the solver left the sketch. They are the
//! solver's starting point next time, so a reopened sketch does not flip to
//! a mirror solution (docs/architecture.md §4). Dimension values are
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

/// What a sketch lies on. Datum planes join this enum with the feature
/// that makes them.
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
}

impl SketchPlane {
    /// The offset along the normal.
    pub fn offset(&self) -> &Expr {
        match self {
            SketchPlane::World { offset, .. } | SketchPlane::Face { offset, .. } => offset,
        }
    }

    pub fn offset_mut(&mut self) -> &mut Expr {
        match self {
            SketchPlane::World { offset, .. } | SketchPlane::Face { offset, .. } => offset,
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
        }
    }

    /// The point entities this one is built on.
    pub fn points(&self) -> Vec<EntityId> {
        match self {
            Geometry::Point { .. } => Vec::new(),
            Geometry::Line { start, end } => vec![*start, *end],
            Geometry::Circle { center, .. } => vec![*center],
            Geometry::Arc { center, start, end } => vec![*center, *start, *end],
            Geometry::Spline { control, .. } => control.clone(),
        }
    }

    /// For a curve with two ends, the points it starts and ends at.
    pub fn ends(&self) -> Option<(EntityId, EntityId)> {
        match self {
            Geometry::Point { .. } | Geometry::Circle { .. } => None,
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
            Geometry::Point { .. } => Role::Point,
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
            "pointOn" => &[&[Point, Line], &[Point, Round]],
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
            self.point(point)?;
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
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = vec![self.plane.offset_mut()];
        out.extend(
            self.constraints
                .iter_mut()
                .filter_map(|c| c.kind.value_mut()),
        );
        out
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
        let mut ids = BTreeSet::new();
        for entity in &self.entities {
            let id = entity.id;
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
            }
            let points = entity.geometry.points();
            for point in &points {
                self.point(*point)?;
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
        Ok(())
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
            ("pointOn", vec![e(1), spline]),
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
}
