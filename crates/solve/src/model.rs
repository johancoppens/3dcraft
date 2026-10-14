//! The solver's input and output: a sketch as plain numbers.

use craft3d_math::Tolerance;

/// Position of an entity in [`Sketch::entities`]. Stable for the caller as
/// long as it keeps its own entity order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

impl EntityId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A sketch entity. Points carry the coordinates; lines and arcs refer to
/// points. The unknowns of a solve are the point coordinates and the circle
/// radii; the values given here are the starting guess.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entity {
    Point {
        x: f64,
        y: f64,
    },
    /// A segment between two points.
    Line {
        start: EntityId,
        end: EntityId,
    },
    /// A full circle around a point.
    Circle {
        center: EntityId,
        radius: f64,
    },
    /// An arc around `center`, counter-clockwise from `start` to `end`. The
    /// solver keeps `|center - start| = |center - end|` as a built-in
    /// equation; the radius of an arc is `|center - start|`.
    Arc {
        center: EntityId,
        start: EntityId,
        end: EntityId,
    },
}

/// What an [`Entity`] is, for error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Point,
    Line,
    Circle,
    Arc,
}

impl Entity {
    pub const fn kind(&self) -> EntityKind {
        match self {
            Entity::Point { .. } => EntityKind::Point,
            Entity::Line { .. } => EntityKind::Line,
            Entity::Circle { .. } => EntityKind::Circle,
            Entity::Arc { .. } => EntityKind::Arc,
        }
    }
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EntityKind::Point => "point",
            EntityKind::Line => "line",
            EntityKind::Circle => "circle",
            EntityKind::Arc => "arc",
        })
    }
}

/// A geometric constraint. Lengths are in model units, angles in radians.
/// "Round" below means a circle or an arc.
///
/// Where a constraint has two mirror solutions (which side of a line, inside
/// or outside a circle, left or right), the side of the starting guess is
/// kept for the whole solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constraint {
    /// Two points at the same place.
    Coincident {
        a: EntityId,
        b: EntityId,
    },
    /// A point on the infinite line through a segment.
    PointOnLine {
        point: EntityId,
        line: EntityId,
    },
    /// A point on a round (on the full circle of an arc).
    PointOnCircle {
        point: EntityId,
        circle: EntityId,
    },
    /// The point lies on the spline `spline` of [`Sketch::splines`]: at no
    /// distance from the curve, or beyond its ends from the line along its
    /// first or last leg. One equation.
    PointOnSpline {
        point: EntityId,
        spline: usize,
    },
    /// A line parallel to the x axis.
    Horizontal {
        line: EntityId,
    },
    /// A line parallel to the y axis.
    Vertical {
        line: EntityId,
    },
    /// Two points with the same y.
    HorizontalPoints {
        a: EntityId,
        b: EntityId,
    },
    /// Two points with the same x.
    VerticalPoints {
        a: EntityId,
        b: EntityId,
    },
    /// Two lines with the same or the opposite direction.
    Parallel {
        a: EntityId,
        b: EntityId,
    },
    Perpendicular {
        a: EntityId,
        b: EntityId,
    },
    /// A line and a round (in either order), or two rounds. The line is
    /// tangent to the full circle; two rounds touch externally or internally,
    /// whichever the starting guess is closer to.
    ///
    /// When the sketch keeps a point on both (a shared end point, or points
    /// tied to both by `Coincident`, `PointOnLine`, `Midpoint` or
    /// `PointOnCircle`), the tangency is taken at that point: the radius to
    /// it is perpendicular to the line, or in line with the other centre.
    Tangent {
        a: EntityId,
        b: EntityId,
    },
    /// Two lines of equal length, or two rounds of equal radius.
    Equal {
        a: EntityId,
        b: EntityId,
    },
    /// Two rounds with the same centre.
    Concentric {
        a: EntityId,
        b: EntityId,
    },
    /// Points `a` and `b` are mirror images in the line `axis`.
    Symmetric {
        a: EntityId,
        b: EntityId,
        axis: EntityId,
    },
    /// A point at the middle of a segment.
    Midpoint {
        point: EntityId,
        line: EntityId,
    },
    /// A point pinned at `at`, or where the starting guess has it (`None`).
    Fixed {
        point: EntityId,
        at: Option<[f64; 2]>,
    },
    /// Distance between two points, or between a point and a line (in either
    /// order). `value >= 0`.
    Distance {
        a: EntityId,
        b: EntityId,
        value: f64,
    },
    /// `|b.x - a.x| = value`, `value >= 0`.
    HorizontalDistance {
        a: EntityId,
        b: EntityId,
        value: f64,
    },
    /// `|b.y - a.y| = value`, `value >= 0`.
    VerticalDistance {
        a: EntityId,
        b: EntityId,
        value: f64,
    },
    /// The counter-clockwise angle from the direction of line `a`
    /// (start to end) to the direction of line `b`, modulo a full turn.
    Angle {
        a: EntityId,
        b: EntityId,
        value: f64,
    },
    /// Radius of a round. `value >= 0`.
    Radius {
        circle: EntityId,
        value: f64,
    },
    /// Diameter of a round. `value >= 0`.
    Diameter {
        circle: EntityId,
        value: f64,
    },
}

/// The solver's input: entities and the constraints on them, in the order
/// they were added. That order decides which constraint is blamed when two
/// cannot hold together: the later one.
/// A polynomial B-spline on control points that are points of the sketch:
/// `control.len() + degree + 1` knots, clamped at both ends, so the curve
/// starts at the first control point and ends at the last. It has no
/// unknowns of its own and is not an entity: only
/// [`Constraint::PointOnSpline`] refers to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    /// From 1 to [`crate::MAX_SPLINE_DEGREE`].
    pub degree: usize,
    pub knots: Vec<f64>,
    pub control: Vec<EntityId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sketch {
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
    /// The splines points are constrained onto.
    pub splines: Vec<Spline>,
}

impl Sketch {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, entity: Entity) -> EntityId {
        let id = EntityId(u32::try_from(self.entities.len()).unwrap_or(u32::MAX));
        self.entities.push(entity);
        id
    }

    pub fn add_point(&mut self, x: f64, y: f64) -> EntityId {
        self.push(Entity::Point { x, y })
    }

    pub fn add_line(&mut self, start: EntityId, end: EntityId) -> EntityId {
        self.push(Entity::Line { start, end })
    }

    pub fn add_circle(&mut self, center: EntityId, radius: f64) -> EntityId {
        self.push(Entity::Circle { center, radius })
    }

    pub fn add_arc(&mut self, center: EntityId, start: EntityId, end: EntityId) -> EntityId {
        self.push(Entity::Arc { center, start, end })
    }

    /// Appends a constraint and returns its index.
    /// Adds a spline for [`Constraint::PointOnSpline`] and returns its
    /// index.
    pub fn add_spline(&mut self, degree: usize, knots: Vec<f64>, control: Vec<EntityId>) -> usize {
        self.splines.push(Spline {
            degree,
            knots,
            control,
        });
        self.splines.len() - 1
    }

    pub fn add_constraint(&mut self, constraint: Constraint) -> usize {
        self.constraints.push(constraint);
        self.constraints.len() - 1
    }

    /// Coordinates of a point entity.
    pub fn point(&self, id: EntityId) -> Option<[f64; 2]> {
        point_of(&self.entities, id)
    }

    /// Radius of a circle, or `|center - start|` of an arc.
    pub fn radius(&self, id: EntityId) -> Option<f64> {
        radius_of(&self.entities, id)
    }
}

pub(crate) fn point_of(entities: &[Entity], id: EntityId) -> Option<[f64; 2]> {
    match entities.get(id.index())? {
        Entity::Point { x, y } => Some([*x, *y]),
        _ => None,
    }
}

pub(crate) fn radius_of(entities: &[Entity], id: EntityId) -> Option<f64> {
    match entities.get(id.index())? {
        Entity::Circle { radius, .. } => Some(*radius),
        Entity::Arc { center, start, .. } => {
            let c = point_of(entities, *center)?;
            let s = point_of(entities, *start)?;
            Some((s[0] - c[0]).hypot(s[1] - c[1]))
        }
        _ => None,
    }
}

/// Knobs of a solve. Nothing here is a global constant: the tolerance comes
/// from the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolveOptions {
    /// A length constraint holds when its residual is at most
    /// `tolerance.linear`; `Parallel`, `Perpendicular` and `Angle` hold within
    /// `tolerance.angular`. Lines shorter than `tolerance.linear` have no
    /// direction.
    pub tolerance: Tolerance,
    /// Cap on the Newton steps of one run. A pass that has to search for
    /// conflicting constraints makes several runs; together they take at
    /// most [`crate::RUNS_PER_SOLVE`] times this many steps. A run cut off
    /// by this cap counts as "cannot be solved", so a cap too low for the
    /// sketch shows up as conflicts or as `DidNotConverge`.
    pub max_iterations: usize,
    /// A constraint equation counts as dependent on the earlier ones when
    /// the part of its (unit-length) gradient that is orthogonal to theirs
    /// is shorter than this. Must be in `(0, 1)`.
    pub rank_threshold: f64,
    /// Cap on the arithmetic of one solve, counted in multiply-adds of the
    /// factorisation. Exceeding it is [`crate::SolveError::WorkLimitExceeded`].
    pub work_limit: u64,
}

impl SolveOptions {
    /// Defaults for everything but the tolerance: 64 steps per run, rank
    /// threshold 1e-8, work limit 5e9 (seconds, not minutes).
    pub const fn new(tolerance: Tolerance) -> Self {
        Self {
            tolerance,
            max_iterations: 64,
            rank_threshold: 1e-8,
            work_limit: 5_000_000_000,
        }
    }
}

/// What the solve found out about one constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintStatus {
    /// Holds, and removes at least one degree of freedom.
    Satisfied,
    /// Holds, but only repeats what earlier constraints already demand.
    Redundant,
    /// Cannot hold together with the constraints added before it, or has
    /// no meaning in the solution of the others (then
    /// [`ConstraintReport::undefined`] says why, and the residual is
    /// infinite). The solution satisfies the others and leaves this one out.
    Conflicting,
}

/// Status and final residual of one constraint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstraintReport {
    pub status: ConstraintStatus,
    /// How far the constraint is from holding in the returned solution,
    /// never negative: a length in model units, or for `Parallel`,
    /// `Perpendicular` (sine of the deviation) and `Angle` an angle in
    /// radians. For constraints with two equations (coincident, concentric,
    /// midpoint, fixed, symmetric, a zero point-point distance) it is the
    /// Euclidean norm of both. Infinite when a conflicting constraint
    /// cannot be evaluated at the solution.
    pub residual: f64,
    /// Why the constraint has no value in the returned solution, when it
    /// has none: it asks for the direction of a line that has no length
    /// there, or for the tangent of an arc without radius. Such a
    /// constraint is always `Conflicting`, with an infinite residual.
    pub undefined: Option<&'static str>,
}

/// The result of a solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// The input entities with solved point coordinates and circle radii.
    /// Feed these back as the starting guess of the next solve.
    pub entities: Vec<Entity>,
    /// Number of unknowns: two per point, one per circle.
    pub unknowns: usize,
    /// Rank of the Jacobian of the non-conflicting constraints at the
    /// solution (arc equations included).
    pub rank: usize,
    /// Degrees of freedom left: `unknowns - rank`.
    pub dof: usize,
    /// One report per input constraint, in input order.
    pub constraints: Vec<ConstraintReport>,
    /// Newton steps taken, over all runs and passes of this solve. Zero for
    /// a sketch that was already solved and has no conflict.
    pub iterations: usize,
}

impl Solution {
    /// Coordinates of a point entity.
    pub fn point(&self, id: EntityId) -> Option<[f64; 2]> {
        point_of(&self.entities, id)
    }

    /// Radius of a circle, or `|center - start|` of an arc.
    pub fn radius(&self, id: EntityId) -> Option<f64> {
        radius_of(&self.entities, id)
    }

    fn with_status(&self, status: ConstraintStatus) -> Vec<usize> {
        self.constraints
            .iter()
            .enumerate()
            .filter(|(_, c)| c.status == status)
            .map(|(i, _)| i)
            .collect()
    }

    /// Indices of the conflicting constraints, ascending.
    pub fn conflicting(&self) -> Vec<usize> {
        self.with_status(ConstraintStatus::Conflicting)
    }

    /// Indices of the redundant constraints, ascending.
    pub fn redundant(&self) -> Vec<usize> {
        self.with_status(ConstraintStatus::Redundant)
    }

    /// No constraint is conflicting.
    pub fn is_consistent(&self) -> bool {
        self.constraints
            .iter()
            .all(|c| c.status != ConstraintStatus::Conflicting)
    }
}
