//! Sketches in regeneration: evaluating the dimensions, handing the sketch
//! to the solver as plain numbers, and reading back how it solved.
//!
//! The coordinates stored in the document are the solver's starting point
//! (the last solution), so a sketch that is already solved does not move,
//! and a changed dimension is followed from where the sketch was rather
//! than from scratch. [`settle_sketches`] writes a new solution back into
//! the document; the engine does that as part of every edit.

use crate::{FeatureResult, Status, angle, length};
use craft3d_doc::{
    ConstraintId, ConstraintKind, DistanceDirection, Document, EntityId, FeatureKind, Geometry,
    NameRole, PersistentName, Sketch, SketchPlane, WorldPlane,
};
use craft3d_expr::Evaluation;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_solve as solve;
use std::collections::BTreeMap;

/// How one constraint came out of the solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintState {
    /// Holds, and takes away at least one degree of freedom.
    Satisfied,
    /// Holds, but only repeats what earlier constraints already demand.
    Redundant,
    /// Cannot hold together with the constraints added before it.
    Conflicting,
    /// The sketch could not be solved at all.
    Unsolved,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstraintResult {
    pub id: ConstraintId,
    pub state: ConstraintState,
    /// How far the constraint is from holding: a length in document units,
    /// or an angle in radians for parallel, perpendicular and angle.
    pub residual: Option<f64>,
}

/// A sketch after solving, placed in space.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchResult {
    /// The sketch's origin and axes in the world.
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
    /// Degrees of freedom left; `None` when the sketch could not be solved.
    pub dof: Option<usize>,
    /// One per constraint, in the order they were added.
    pub constraints: Vec<ConstraintResult>,
    /// Solved coordinates of the points, in sketch axes.
    pub points: Vec<(EntityId, f64, f64)>,
    /// Solved radii of the circles.
    pub radii: Vec<(EntityId, f64)>,
}

impl SketchResult {
    pub fn point(&self, id: EntityId) -> Option<(f64, f64)> {
        self.points
            .iter()
            .find(|(point, ..)| *point == id)
            .map(|&(_, x, y)| (x, y))
    }

    pub fn radius(&self, id: EntityId) -> Option<f64> {
        self.radii
            .iter()
            .find(|(circle, _)| *circle == id)
            .map(|&(_, r)| r)
    }

    /// A point of the sketch in world coordinates.
    pub fn to_world(&self, x: f64, y: f64) -> Vec3 {
        self.origin + self.x_axis * x + self.y_axis * y
    }
}

/// The axes of a world plane: sketch x, sketch y and the normal.
fn axes(plane: WorldPlane) -> (Vec3, Vec3, Vec3) {
    match plane {
        WorldPlane::Xy => (Vec3::X, Vec3::Y, Vec3::Z),
        WorldPlane::Yz => (Vec3::Y, Vec3::Z, Vec3::X),
        WorldPlane::Zx => (Vec3::Z, Vec3::X, Vec3::Y),
    }
}

fn list(ids: &[ConstraintId]) -> String {
    let shown: Vec<String> = ids.iter().take(10).map(|id| id.to_string()).collect();
    let more = ids.len().saturating_sub(shown.len());
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// The sketch as the solver takes it, with the way back to document ids.
struct Problem {
    input: solve::Sketch,
    /// Document id of each solver entity, by solver index.
    entities: Vec<EntityId>,
    /// Document id of each solver constraint, by solver index.
    constraints: Vec<ConstraintId>,
}

fn problem(doc: &Document, variables: &Evaluation, sketch: &Sketch) -> Result<Problem, String> {
    let mut index = BTreeMap::new();
    for (i, entity) in sketch.entities.iter().enumerate() {
        let i = u32::try_from(i).map_err(|_| "the sketch has too many entities".to_string())?;
        index.insert(entity.id, solve::EntityId(i));
    }
    let find = |id: EntityId| -> Result<solve::EntityId, String> {
        index
            .get(&id)
            .copied()
            .ok_or_else(|| format!("there is no entity {id}"))
    };
    let mut input = solve::Sketch::new();
    for entity in &sketch.entities {
        input.entities.push(match entity.geometry {
            Geometry::Point { x, y } => solve::Entity::Point { x, y },
            Geometry::Line { start, end } => solve::Entity::Line {
                start: find(start)?,
                end: find(end)?,
            },
            Geometry::Circle { center, radius } => solve::Entity::Circle {
                center: find(center)?,
                radius,
            },
            Geometry::Arc { center, start, end } => solve::Entity::Arc {
                center: find(center)?,
                start: find(start)?,
                end: find(end)?,
            },
        });
    }
    let is_line = |id: EntityId| {
        sketch
            .entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Line { .. }))
    };
    let ends = |id: EntityId| -> Result<(solve::EntityId, solve::EntityId), String> {
        match sketch.entity(id).map(|e| e.geometry) {
            Ok(Geometry::Line { start, end }) => Ok((find(start)?, find(end)?)),
            _ => Err(format!("{id} is not a line")),
        }
    };
    for constraint in &sketch.constraints {
        let id = constraint.id;
        let name = format!("{id}.value");
        use solve::Constraint as S;
        let mapped = match &constraint.kind {
            ConstraintKind::Coincident { a, b } => S::Coincident {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::PointOn { point, curve } => {
                if is_line(*curve) {
                    S::PointOnLine {
                        point: find(*point)?,
                        line: find(*curve)?,
                    }
                } else {
                    S::PointOnCircle {
                        point: find(*point)?,
                        circle: find(*curve)?,
                    }
                }
            }
            ConstraintKind::Horizontal { a, b: None } => S::Horizontal { line: find(*a)? },
            ConstraintKind::Horizontal { a, b: Some(b) } => S::HorizontalPoints {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Vertical { a, b: None } => S::Vertical { line: find(*a)? },
            ConstraintKind::Vertical { a, b: Some(b) } => S::VerticalPoints {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Parallel { a, b } => S::Parallel {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Perpendicular { a, b } => S::Perpendicular {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Tangent { a, b } => S::Tangent {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Equal { a, b } => S::Equal {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Concentric { a, b } => S::Concentric {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Symmetric { a, b, about } => S::Symmetric {
                a: find(*a)?,
                b: find(*b)?,
                axis: find(*about)?,
            },
            ConstraintKind::Midpoint { point, line } => S::Midpoint {
                point: find(*point)?,
                line: find(*line)?,
            },
            ConstraintKind::Fixed { point } => S::Fixed {
                point: find(*point)?,
                at: None,
            },
            ConstraintKind::Distance {
                a,
                b,
                direction,
                value,
            } => {
                let value = length(doc, variables, &name, value)?;
                // The length of a line is the distance between its ends.
                let (a, b) = match b {
                    Some(b) => (find(*a)?, find(*b)?),
                    None => ends(*a)?,
                };
                match direction {
                    DistanceDirection::Direct => S::Distance { a, b, value },
                    DistanceDirection::Horizontal => S::HorizontalDistance { a, b, value },
                    DistanceDirection::Vertical => S::VerticalDistance { a, b, value },
                }
            }
            ConstraintKind::Angle { a, b, value } => S::Angle {
                a: find(*a)?,
                b: find(*b)?,
                value: angle(doc, variables, &name, value)?,
            },
            ConstraintKind::Radius { curve, value } => S::Radius {
                circle: find(*curve)?,
                value: length(doc, variables, &name, value)?,
            },
            ConstraintKind::Diameter { curve, value } => S::Diameter {
                circle: find(*curve)?,
                value: length(doc, variables, &name, value)?,
            },
        };
        input.constraints.push(mapped);
    }
    Ok(Problem {
        input,
        entities: sketch.entities.iter().map(|e| e.id).collect(),
        constraints: sketch.constraints.iter().map(|c| c.id).collect(),
    })
}

/// A solver error in the document's own ids.
fn explain(error: &solve::SolveError, problem: &Problem) -> String {
    use solve::{Site, SolveError as E};
    let constraint = |i: usize| {
        problem
            .constraints
            .get(i)
            .map_or_else(|| format!("constraint {i}"), |id| id.to_string())
    };
    let entity = |i: usize| {
        problem
            .entities
            .get(i)
            .map_or_else(|| format!("entity {i}"), |id| id.to_string())
    };
    let site = |at: &Site| match at {
        Site::Entity(i) => entity(*i),
        Site::Constraint(i) => constraint(*i),
    };
    match error {
        E::UnknownEntity { at, id } => {
            format!(
                "{} refers to {}, which does not exist",
                site(at),
                entity(id.index())
            )
        }
        E::WrongKind {
            at,
            id,
            expected,
            found,
        } => format!(
            "{} needs {} to be a {expected}, but it is a {found}",
            site(at),
            entity(id.index())
        ),
        E::NonFinite { at } => format!("{} has a value that is not finite", site(at)),
        E::InvalidValue {
            constraint: i,
            reason,
        } => {
            format!("{} has a value that cannot be: {reason}", constraint(*i))
        }
        E::Degenerate {
            constraint: i,
            reason,
        } => {
            format!("{} has no meaning here: {reason}", constraint(*i))
        }
        E::Unsupported {
            constraint: i,
            what,
        } => {
            format!("{} is not supported yet: {what}", constraint(*i))
        }
        other => other.to_string(),
    }
}

/// Coordinates of points, by entity.
type Points = Vec<(EntityId, f64, f64)>;
/// Radii of circles, by entity.
type Radii = Vec<(EntityId, f64)>;

/// The stored coordinates, for a sketch that could not be solved.
fn stored(sketch: &Sketch) -> (Points, Radii) {
    let mut points = Vec::new();
    let mut radii = Vec::new();
    for entity in &sketch.entities {
        match entity.geometry {
            Geometry::Point { x, y } => points.push((entity.id, x, y)),
            Geometry::Circle { radius, .. } => radii.push((entity.id, radius)),
            Geometry::Line { .. } | Geometry::Arc { .. } => {}
        }
    }
    (points, radii)
}

/// Where a sketch lies in space: its origin, its two axes and its normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
}

impl Placement {
    /// The world xy plane: for solving a sketch without placing it.
    pub(crate) const FLAT: Placement = Placement {
        origin: Vec3::ZERO,
        x_axis: Vec3::X,
        y_axis: Vec3::Y,
        normal: Vec3::Z,
    };
}

/// The face a name points at, among the features regenerated so far.
/// A name that points at nothing is an error that says what is there
/// instead; it never resolves to another face.
fn resolve_face(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
) -> Result<Frame, String> {
    let Some(source) = doc.feature(name.feature) else {
        return Err(format!(
            "plane: there is no feature {} for the face {name} (was it deleted?)",
            name.feature
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == name.feature) else {
        return Err(format!(
            "plane: `{made_by}`, which makes the face {name}, comes after this sketch in the history; move it before (feature.move)"
        ));
    };
    let (Some(body), Some(origins)) = (&outcome.body, &outcome.origins) else {
        return Err(match &outcome.status {
            Status::Ok => format!(
                "plane: `{made_by}` is a {}, whose faces have no names yet; only extrusions name their faces",
                source.kind.type_name()
            ),
            Status::Suppressed => {
                format!("plane: `{made_by}`, which makes the face {name}, is suppressed")
            }
            Status::RolledBack => {
                format!("plane: `{made_by}`, which makes the face {name}, is rolled back")
            }
            Status::Failed(_) => {
                format!("plane: `{made_by}`, which makes the face {name}, failed; fix it first")
            }
        });
    };
    if !name.role.is_face() {
        return Err(format!(
            "plane: {name} is not a face; a sketch lies on a startCap, an endCap or a side"
        ));
    }
    let Some(index) = origins
        .faces
        .iter()
        .position(|origin| crate::extrude::name_of(name.feature, origin) == *name)
    else {
        let there: Vec<String> = origins
            .faces
            .iter()
            .take(12)
            .map(|origin| crate::extrude::name_of(name.feature, origin).to_string())
            .collect();
        return Err(format!(
            "plane: `{made_by}` no longer has a face {name}; its faces are: {}{}",
            there.join(", "),
            if origins.faces.len() > there.len() {
                ", ..."
            } else {
                ""
            }
        ));
    };
    let Some(face) = body.faces.get(index) else {
        return Err(format!("plane: the face {name} is missing from its body"));
    };
    let craft3d_geom::Surface::Plane(plane) = &face.surface else {
        return Err(format!(
            "plane: the face {name} is a {} face; a sketch needs a flat one",
            face.surface.kind()
        ));
    };
    let frame = plane.frame();
    // A side face of an extrusion starts wherever the extrusion starts,
    // which moves with its distance and direction. The sketch's origin is
    // put back in the plane of the extruded sketch, so a sketch on a side
    // keeps its place when the extrusion gets deeper, is turned round or
    // made symmetric.
    let mut origin = frame.origin();
    if name.role == NameRole::Side
        && let FeatureKind::Extrude(extrusion) = &source.kind
        && let Some(base) = earlier
            .iter()
            .find(|r| r.id == extrusion.sketch)
            .and_then(|r| r.sketch.as_ref())
    {
        origin = origin - base.normal * (origin - base.origin).dot(base.normal);
    }
    if face.reversed {
        frame.reversed_at(origin)
    } else {
        frame.at(origin)
    }
    .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))
}

/// Where the sketch lies: its plane with the offset applied.
pub(crate) fn place(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    earlier: &[FeatureResult],
) -> Result<Placement, String> {
    let offset = length(doc, variables, "plane.offset", sketch.plane.offset())?;
    if !offset.is_finite() {
        return Err("plane.offset is not finite".into());
    }
    match &sketch.plane {
        SketchPlane::World { plane, .. } => {
            let (x_axis, y_axis, normal) = axes(*plane);
            Ok(Placement {
                origin: normal * offset,
                x_axis,
                y_axis,
                normal,
            })
        }
        SketchPlane::Face { face, .. } => {
            let frame = resolve_face(doc, earlier, face)?;
            Ok(Placement {
                origin: frame.origin() + frame.z() * offset,
                x_axis: frame.x(),
                y_axis: frame.y(),
                normal: frame.z(),
            })
        }
    }
}

/// Solves one sketch from its stored coordinates. The result is `None`
/// only when the plane itself could not be found.
pub(crate) fn solve_sketch(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    placement: Result<Placement, String>,
) -> (Status, Option<SketchResult>) {
    let Placement {
        origin,
        x_axis,
        y_axis,
        normal,
    } = match placement {
        Ok(placement) => placement,
        Err(reason) => return (Status::Failed(reason), None),
    };
    let (points, radii) = stored(sketch);
    let unsolved = |reason: String| {
        let result = SketchResult {
            origin,
            x_axis,
            y_axis,
            normal,
            dof: None,
            constraints: sketch
                .constraints
                .iter()
                .map(|c| ConstraintResult {
                    id: c.id,
                    state: ConstraintState::Unsolved,
                    residual: None,
                })
                .collect(),
            points: points.clone(),
            radii: radii.clone(),
        };
        (Status::Failed(reason), Some(result))
    };
    let problem = match problem(doc, variables, sketch) {
        Ok(problem) => problem,
        Err(reason) => return unsolved(reason),
    };
    let Some(tolerance) = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular) else {
        return unsolved("the document's tolerance is not positive and finite".into());
    };
    let solution = match solve::solve(&problem.input, &solve::SolveOptions::new(tolerance)) {
        Ok(solution) => solution,
        Err(error) => return unsolved(explain(&error, &problem)),
    };

    let mut points = Vec::new();
    let mut radii = Vec::new();
    for (id, entity) in problem.entities.iter().zip(&solution.entities) {
        match *entity {
            solve::Entity::Point { x, y } => points.push((*id, x, y)),
            solve::Entity::Circle { radius, .. } => radii.push((*id, radius)),
            solve::Entity::Line { .. } | solve::Entity::Arc { .. } => {}
        }
    }
    let constraints: Vec<ConstraintResult> = problem
        .constraints
        .iter()
        .zip(&solution.constraints)
        .map(|(id, report)| ConstraintResult {
            id: *id,
            state: match report.status {
                solve::ConstraintStatus::Satisfied => ConstraintState::Satisfied,
                solve::ConstraintStatus::Redundant => ConstraintState::Redundant,
                solve::ConstraintStatus::Conflicting => ConstraintState::Conflicting,
            },
            residual: Some(report.residual),
        })
        .collect();
    let conflicting: Vec<ConstraintId> = constraints
        .iter()
        .filter(|c| c.state == ConstraintState::Conflicting)
        .map(|c| c.id)
        .collect();
    // Why a blamed constraint has no value at all where the others put
    // the sketch (an angle on a line that has shrunk to a point).
    let undefined = |id: &ConstraintId| {
        problem
            .constraints
            .iter()
            .zip(&solution.constraints)
            .find(|(c, _)| *c == id)
            .and_then(|(_, report)| report.undefined)
    };
    let status = match conflicting.as_slice() {
        [] => Status::Ok,
        [one] => Status::Failed(match undefined(one) {
            Some(reason) => format!(
                "constraint {one} has no meaning where the constraints before it put the sketch: {reason}; change or delete it"
            ),
            None => format!(
                "constraint {one} conflicts with the constraints before it; change or delete it"
            ),
        }),
        many => Status::Failed(format!(
            "constraints {} conflict with the constraints before them; change or delete them",
            list(many)
        )),
    };
    let result = SketchResult {
        origin,
        x_axis,
        y_axis,
        normal,
        dof: Some(solution.dof),
        constraints,
        points,
        radii,
    };
    (status, Some(result))
}

/// Solves every sketch and stores the solution in the document, so the
/// next solve starts from it. A sketch that cannot be solved, or only with
/// a constraint left out as conflicting, keeps its coordinates. Returns
/// true when anything moved.
pub fn settle_sketches(doc: &mut Document) -> bool {
    let Ok(table) = doc.variable_table() else {
        return false;
    };
    let variables = table.evaluate();
    let mut solved = Vec::new();
    for (index, feature) in doc.features.iter().enumerate() {
        if let FeatureKind::Sketch(sketch) = &feature.kind
            && !feature.suppressed
            // Where the sketch lies does not change how it solves.
            && let (_, Some(result)) = solve_sketch(doc, &variables, sketch, Ok(Placement::FLAT))
            && result.dof.is_some()
            // A solution with a conflict in it is not a place to start
            // from: it may have lost what the sketch looked like (a line
            // shrunk to a point no longer says which way it pointed). The
            // sketch keeps its last good coordinates until the conflict is
            // gone, and then comes back on the side it was on.
            && result
                .constraints
                .iter()
                .all(|c| c.state != ConstraintState::Conflicting)
        {
            solved.push((index, result));
        }
    }
    let mut moved = false;
    for (index, result) in solved {
        let Some(FeatureKind::Sketch(sketch)) = doc.features.get_mut(index).map(|f| &mut f.kind)
        else {
            continue;
        };
        for entity in &mut sketch.entities {
            match &mut entity.geometry {
                Geometry::Point { x, y } => {
                    if let Some((sx, sy)) = result.point(entity.id)
                        && sx.is_finite()
                        && sy.is_finite()
                        && (*x, *y) != (sx, sy)
                    {
                        (*x, *y) = (sx, sy);
                        moved = true;
                    }
                }
                Geometry::Circle { radius, .. } => {
                    if let Some(r) = result.radius(entity.id)
                        && r.is_finite()
                        && r > 0.0
                        && *radius != r
                    {
                        *radius = r;
                        moved = true;
                    }
                }
                Geometry::Line { .. } | Geometry::Arc { .. } => {}
            }
        }
    }
    moved
}
