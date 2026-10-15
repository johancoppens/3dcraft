//! Sketches in regeneration: evaluating the dimensions, handing the sketch
//! to the solver as plain numbers, and reading back how it solved.
//!
//! The coordinates stored in the document are the solver's starting point
//! (the last solution), so a sketch that is already solved does not move,
//! and a changed dimension is followed from where the sketch was rather
//! than from scratch. [`settle_sketches`] writes a new solution back into
//! the document; the engine does that as part of every edit.

use crate::cause::FIX_IT_FIRST;
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
    /// Document id of each solver entity, by solver index: every entity
    /// but the splines, in the sketch's order, and then for each helper
    /// line that stands for the end of a spline in a tangency the id of
    /// that spline.
    entities: Vec<EntityId>,
    /// Document id of each solver constraint, by solver index.
    constraints: Vec<ConstraintId>,
}

fn problem(doc: &Document, variables: &Evaluation, sketch: &Sketch) -> Result<Problem, String> {
    // A spline is not an entity of the solver's: its shape is that of its
    // control points, which are. So it adds no unknowns and no equations,
    // and the degrees of freedom are those of the points.
    let mut index = BTreeMap::new();
    let mut entities = Vec::new();
    let position = |count: usize| {
        u32::try_from(count)
            .map(solve::EntityId)
            .map_err(|_| "the sketch has too many entities".to_string())
    };
    for entity in &sketch.entities {
        if matches!(entity.geometry, Geometry::Spline { .. }) {
            continue;
        }
        index.insert(entity.id, position(entities.len())?);
        entities.push(entity.id);
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
            Geometry::Spline { .. } => continue,
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
    // What a curve gives a tangency that involves a spline: a line to be
    // parallel to, or a circle or arc to touch.
    enum Along {
        Line(solve::EntityId),
        Round(solve::EntityId),
    }
    // A clamped spline leaves its end point along the leg to the next
    // control point. For the solver that leg is a line between two of its
    // points, added here: it has no unknowns of its own, and stands for
    // the spline in messages.
    let along = |input: &mut solve::Sketch,
                 entities: &mut Vec<EntityId>,
                 curve: EntityId,
                 at: EntityId|
     -> Result<Along, String> {
        match &sketch.entity(curve).map_err(|e| e.to_string())?.geometry {
            Geometry::Line { .. } => Ok(Along::Line(find(curve)?)),
            Geometry::Circle { .. } | Geometry::Arc { .. } => Ok(Along::Round(find(curve)?)),
            Geometry::Point { .. } => Err(format!("{curve} is a point, which has no direction")),
            Geometry::Spline { control, .. } => {
                let leg = if control.first() == Some(&at) {
                    control.get(1)
                } else {
                    control.len().checked_sub(2).and_then(|i| control.get(i))
                };
                let Some(next) = leg else {
                    return Err(format!("{curve} has too few control points"));
                };
                let id = position(input.entities.len())?;
                input.entities.push(solve::Entity::Line {
                    start: find(at)?,
                    end: find(*next)?,
                });
                entities.push(curve);
                Ok(Along::Line(id))
            }
        }
    };
    let is_spline = |id: EntityId| {
        sketch
            .entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Spline { .. }))
    };
    let is_line = |id: EntityId| {
        sketch
            .entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Line { .. }))
    };
    let ends = |id: EntityId| -> Result<(solve::EntityId, solve::EntityId), String> {
        match sketch.entity(id).map(|e| &e.geometry) {
            Ok(&Geometry::Line { start, end }) => Ok((find(start)?, find(end)?)),
            _ => Err(format!("{id} is not a line")),
        }
    };
    let mut splines: BTreeMap<EntityId, usize> = BTreeMap::new();
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
                if let Ok(Geometry::Spline { degree, control }) =
                    sketch.entity(*curve).map(|e| &e.geometry)
                {
                    // The solver has the spline as its control points, in
                    // a list of its own; each spline is given once.
                    let number = match splines.get(curve) {
                        Some(number) => *number,
                        None => {
                            let points = control
                                .iter()
                                .map(|id| find(*id))
                                .collect::<Result<Vec<_>, _>>()?;
                            let number = input.add_spline(
                                usize::try_from(*degree).unwrap_or(usize::MAX),
                                craft3d_doc::clamped_uniform_knots(*degree, control.len()),
                                points,
                            );
                            splines.insert(*curve, number);
                            number
                        }
                    };
                    S::PointOnSpline {
                        point: find(*point)?,
                        spline: number,
                    }
                } else if is_line(*curve) {
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
            ConstraintKind::Tangent { a, b } if is_spline(*a) || is_spline(*b) => {
                let Some((at_a, at_b)) = sketch.tangent_at(*a, *b) else {
                    return Err(format!(
                        "{id}: a spline is tangent to another curve at an end point the two share, and {a} and {b} share none (any more); make their ends coincident, or delete the constraint"
                    ));
                };
                let first = along(&mut input, &mut entities, *a, at_a)?;
                let second = along(&mut input, &mut entities, *b, at_b)?;
                match (first, second) {
                    (Along::Line(a), Along::Line(b)) => S::Parallel { a, b },
                    // The solver takes the tangency at the point both are
                    // kept on, which is the shared end.
                    (Along::Line(line), Along::Round(round))
                    | (Along::Round(round), Along::Line(line)) => S::Tangent { a: line, b: round },
                    (Along::Round(_), Along::Round(_)) => {
                        return Err(format!("{id}: neither {a} nor {b} is a spline"));
                    }
                }
            }
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
        entities,
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
            Geometry::Line { .. } | Geometry::Arc { .. } | Geometry::Spline { .. } => {}
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

/// The feature a name says made the entity, with its result: it has to
/// exist, come earlier, and have a body (taken by a later feature or not).
fn maker<'a>(
    doc: &'a Document,
    earlier: &'a [FeatureResult],
    feature: craft3d_doc::FeatureId,
    name: &PersistentName,
) -> Result<
    (
        &'a craft3d_doc::Feature,
        &'a FeatureResult,
        &'a craft3d_topo::Body,
    ),
    String,
> {
    let Some(source) = doc.feature(feature) else {
        return Err(format!(
            "plane: there is no feature {feature} for the face {name} (was it deleted?)"
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == feature) else {
        return Err(format!(
            "plane: `{made_by}`, which makes the face {name}, {}",
            crate::comes_after(doc, earlier, feature)
        ));
    };
    let Some(body) = &outcome.body else {
        return Err(match &outcome.status {
            Status::Ok => format!(
                "plane: `{made_by}` is a {}, whose faces have no names yet",
                source.kind.type_name()
            ),
            Status::Suppressed => {
                format!("plane: `{made_by}`, which makes the face {name}, is suppressed")
            }
            Status::RolledBack => {
                format!("plane: `{made_by}`, which makes the face {name}, is rolled back")
            }
            Status::Failed(_) => {
                format!("plane: `{made_by}`, which makes the face {name}, {FIX_IT_FIRST}")
            }
        });
    };
    Ok((source, outcome, body))
}

/// "`Plate` no longer has a face X; its faces are: ...".
fn no_such_face(made_by: &str, name: &PersistentName, names: &[PersistentName]) -> String {
    let there: Vec<String> = names.iter().take(12).map(ToString::to_string).collect();
    format!(
        "plane: `{made_by}` no longer has a face {name}; its faces are: {}{}",
        there.join(", "),
        if names.len() > there.len() {
            ", ..."
        } else {
            ""
        }
    )
}

/// True when two face names are the same, or differ only in the outline
/// their cap was named after: all caps of one role of one feature lie in
/// one plane (see [`made_face`]).
pub(crate) fn same_plane(a: &PersistentName, b: &PersistentName) -> bool {
    let (mut a, mut b) = (a, b);
    for _ in 0..craft3d_doc::MAX_NAME_DEPTH {
        match (&a.of, &b.of) {
            (Some(next_a), Some(next_b)) => {
                if (a.feature, a.role, &a.sources) != (b.feature, b.role, &b.sources) {
                    return false;
                }
                (a, b) = (next_a, next_b);
            }
            (None, None) => {
                return a.feature == b.feature
                    && a.role == b.role
                    && (a.sources == b.sources
                        || matches!(a.role, NameRole::StartCap | NameRole::EndCap));
            }
            _ => return false,
        }
    }
    false
}

/// The frame of the face a name points at, among the features regenerated
/// so far. A name that points at nothing is an error that says what is
/// there instead; it never resolves to another face.
pub(crate) fn resolve_face(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
) -> Result<Frame, String> {
    if !name.is_face() {
        return Err(format!(
            "plane: {name} is not a face; a sketch lies on a startCap, an endCap or a side"
        ));
    }
    // A copy of a face lies where the face it is a copy of lies, moved by
    // the feature that copied it. So the way of the copy is walked down to
    // the face as it was first made, feature by feature, and its frame is
    // carried back up.
    let mut maps = Vec::new();
    let mut whole = name.clone();
    let mut rounds = 0usize;
    while whole.role == NameRole::Copy {
        // A name in the short form is one copy; one built by hand may be
        // a copy of a copy.
        rounds = rounds.saturating_add(1);
        if rounds > craft3d_doc::MAX_NAME_DEPTH {
            return Err(format!("plane: {name} is made of too many names"));
        }
        let (source, outcome, _) = maker(doc, earlier, whole.feature, name)?;
        let listed = outcome.names.as_ref().map_or(&[][..], |n| &n.faces[..]);
        // The face has to be one of the body each instance on the way was
        // made from (for a boolean: instance 0 is the target, 1 the tool).
        let Some((steps, end)) = crate::follow::steps(doc, earlier, &whole) else {
            return Err(no_such_face(&source.name, &whole, listed));
        };
        // What the face is called in the body of each feature on the way,
        // from the innermost out: each a copy of the next.
        let mut called = Vec::with_capacity(steps.len());
        let mut inner = end.clone();
        for &(feature, instance) in steps.iter().rev() {
            inner = PersistentName::copy(feature, instance, inner);
            called.push(inner.clone());
        }
        called.reverse();
        for (&(feature, instance), link) in steps.iter().zip(&called) {
            walk_copy(doc, earlier, name, feature, instance, link, &mut maps)?;
        }
        whole = end.clone();
    }
    let mut frame = made_face(doc, earlier, &whole, name)?;
    for (map, moved, turned) in maps.iter().rev() {
        // Seen from outside the moved body: z is the image of the normal
        // and x the image of x. After a mirror that leaves y pointing the
        // other way than the image of y, as a right-handed frame must.
        frame = Frame::new(
            map.point(frame.origin()),
            map.vector(frame.z()),
            map.vector(frame.x()),
        )
        .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        if *turned {
            // Seen from the other side: x stays, y and z turn round, as
            // for a face whose outward normal is against its surface's.
            frame = frame
                .reversed_at(frame.origin())
                .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        }
        if let Some((on, outward, hollow, turns)) = moved {
            if *turns {
                // Turned the shortest way onto the face's new normal:
                // about the line the old and the new plane share, which
                // is the face's own turn for every turn that tilts it.
                frame = turned_onto(&frame, *outward)
                    .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
            }
            // The origin dropped straight onto the plane the face lies in
            // now; the axes stay what they were. The face of a hollow
            // looks into it: its z is the face's own normal and its x
            // stays, so its y is the other way round, as when a wall is
            // looked at from behind.
            let z = if *hollow { *outward } else { frame.z() };
            let origin = frame.origin() + z * (*on - frame.origin()).dot(z);
            frame = if *hollow {
                Frame::new(origin, z, frame.x())
            } else {
                frame.at(origin)
            }
            .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        }
    }
    Ok(frame)
}

/// `frame` turned the shortest way so that its z is `normal` (a unit
/// vector): its x and y turn with it about the line square to both.
/// The frame itself when z is `normal` already.
fn turned_onto(frame: &Frame, normal: Vec3) -> Option<Frame> {
    let z = frame.z();
    let across = z.cross(normal);
    let sine = across.length();
    let cosine = z.dot(normal);
    if sine <= 1e-15 && cosine > 0.0 {
        return Some(*frame);
    }
    let turn = match across.normalized() {
        Some(axis) => craft3d_math::Transform::rotation(axis, sine.atan2(cosine))?,
        // Turned right round: about x.
        None => craft3d_math::Transform::rotation(frame.x(), std::f64::consts::PI)?,
    };
    Frame::new(frame.origin(), normal, turn.vector(frame.x()))
}

/// What one step of the way of a copy does to a face's frame: the map of
/// its instance, where a face operation moved the face (the point on its
/// plane, its outward normal, whether it is the face of a hollow, and
/// whether the face may have turned), and whether a subtraction turned it
/// inside out.
type Carried = (
    craft3d_math::Transform,
    Option<(Vec3, Vec3, bool, bool)>,
    bool,
);

/// One step of the way of a copy (see [`resolve_face`]): `link` is what
/// the face is called in the body of `feature`, where it is in instance
/// `instance`. Pushes the map of that instance, and how the feature moved
/// or turned the face, onto `maps`.
fn walk_copy(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    feature: craft3d_doc::FeatureId,
    instance: u64,
    link: &PersistentName,
    maps: &mut Vec<Carried>,
) -> Result<(), String> {
    let (source, outcome, _) = maker(doc, earlier, feature, name)?;
    let made_by = &source.name;
    let listed = outcome.names.as_ref().map_or(&[][..], |n| &n.faces[..]);
    let Some(instances) = &outcome.instances else {
        return Err(no_such_face(made_by, link, listed));
    };
    let instance_map = usize::try_from(instance)
        .ok()
        .and_then(|i| instances.get(i));
    let Some(map) = instance_map else {
        return Err(format!(
            "plane: `{made_by}` has {} instance(s), numbered from 0; there is no {link}",
            instances.len()
        ));
    };
    // By its own name or an alias: a face a boolean merged from faces
    // of both operands answers to the name of each.
    let names = outcome.names.as_ref();
    let found =
        names.is_some_and(|names| names.all_face_names().any(|face| same_plane(face, link)));
    if !found {
        return Err(no_such_face(made_by, link, listed));
    }
    // A face operation moves faces one by one, which no map of the
    // whole body says: the face is found where it lies now.
    let moved = match source.kind {
        FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Shell(_) => {
            let index = names.and_then(|names| {
                (0..names.faces.len())
                    .find(|&face| names.names_of(face).any(|own| same_plane(own, link)))
            });
            let face = index.and_then(|index| outcome.body.as_ref()?.faces.get(index));
            match face.map(|face| (&face.surface, face.reversed)) {
                Some((craft3d_geom::Surface::Plane(plane), reversed)) => {
                    let outward = if reversed {
                        -plane.frame().z()
                    } else {
                        plane.frame().z()
                    };
                    // The faces of a shell's hollow (instance 1)
                    // look the other way than the faces they lie
                    // behind.
                    // So do the faces a layer has from under the faces
                    // it thickens.
                    let hollow =
                        matches!(source.kind, FeatureKind::Shell(_) | FeatureKind::Thicken(_))
                            && instance == 1;
                    // A moved or drafted face may also have turned.
                    let turns = matches!(
                        source.kind,
                        FeatureKind::MoveFace(_) | FeatureKind::Draft(_)
                    );
                    Some((plane.frame().origin(), outward, hollow, turns))
                }
                Some((surface, _)) => {
                    return Err(format!(
                        "plane: the face {name} is a {} face; a sketch needs a flat one",
                        surface.kind()
                    ));
                }
                None => return Err(no_such_face(made_by, link, listed)),
            }
        }
        _ => None,
    };
    // A subtraction keeps the faces of its tool turned inside out:
    // the floor of a pocket looks up out of the material, where the
    // bottom of the cutter it was looked down. A sketch on it looks
    // out of the material, as on every other face.
    let turned = matches!(
        source.kind,
        FeatureKind::Boolean(craft3d_doc::Boolean {
            op: craft3d_doc::BooleanOp::Subtract,
            ..
        })
    ) && instance == 1;
    maps.push((*map, moved, turned));
    Ok(())
}

/// The frame of a face of the feature that first made it: `link` is the
/// end of the chain of `whole`.
fn made_face(
    doc: &Document,
    earlier: &[FeatureResult],
    link: &PersistentName,
    whole: &PersistentName,
) -> Result<Frame, String> {
    let (source, outcome, body) = maker(doc, earlier, link.feature, whole)?;
    let made_by = &source.name;
    let Some(names) = &outcome.names else {
        return Err(format!(
            "plane: `{made_by}` is a {}, whose faces have no names yet (the primitives do not name their faces)",
            source.kind.type_name()
        ));
    };
    let exact = names.faces.iter().position(|face| face == link);
    // The caps of one role all lie in one plane and share one frame (that
    // of the extruded sketch), whatever their outlines are. So when the
    // outline a cap was named after has changed, the sketch still knows
    // its plane: that of any cap of the same role of the same feature.
    // A side face has no such fallback: its plane is its own.
    let same_plane = || {
        matches!(link.role, NameRole::StartCap | NameRole::EndCap)
            .then(|| names.faces.iter().position(|face| face.role == link.role))
            .flatten()
    };
    let Some(index) = exact.or_else(same_plane) else {
        return Err(no_such_face(made_by, link, &names.faces));
    };
    let Some(face) = body.faces.get(index) else {
        return Err(format!("plane: the face {link} is missing from its body"));
    };
    let craft3d_geom::Surface::Plane(plane) = &face.surface else {
        return Err(format!(
            "plane: the face {link} is a {} face; a sketch needs a flat one",
            face.surface.kind()
        ));
    };
    let frame = plane.frame();
    // The kernel puts the origin of a side face where its line starts, at
    // the offset the extrusion starts. Both move with upstream dimensions
    // (a corner radius, the distance, the direction). A sketch on a side
    // is measured from the origin of the extruded sketch instead, dropped
    // straight onto the face: a point that only moves when the face itself
    // moves along its normal.
    let mut origin = frame.origin();
    if link.root().role == NameRole::Side
        && let FeatureKind::Extrude(extrusion) = &source.kind
        && let Some(base) = earlier
            .iter()
            .find(|r| r.id == extrusion.sketch)
            .and_then(|r| r.sketch.as_ref())
    {
        origin = base.origin - frame.z() * (base.origin - origin).dot(frame.z());
    }
    if face.reversed {
        frame.reversed_at(origin)
    } else {
        frame.at(origin)
    }
    .ok_or_else(|| format!("plane: the face {link} has a degenerate plane"))
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
        SketchPlane::Datum { plane, .. } => {
            let frame = crate::datum::datum_plane(doc, earlier, *plane, "plane")?;
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
    settle_sketches_cached(doc, &mut crate::RegenCache::default())
}

/// [`settle_sketches`], solving only the sketches that have changed since
/// `cache` last saw them solved. The outcome is the same.
pub fn settle_sketches_cached(doc: &mut Document, cache: &mut crate::RegenCache) -> bool {
    let Ok(table) = doc.variable_table() else {
        return false;
    };
    let variables = table.evaluate();
    let mut solved = Vec::new();
    for (index, feature) in doc.features.iter().enumerate() {
        if let FeatureKind::Sketch(sketch) = &feature.kind
            && !feature.suppressed
            // Where the sketch lies does not change how it solves.
            && let (_, Some(result)) = cache.solve(feature.id, doc, &variables, sketch)
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
                Geometry::Line { .. } | Geometry::Arc { .. } | Geometry::Spline { .. } => {}
            }
        }
    }
    moved
}
