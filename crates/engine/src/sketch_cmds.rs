//! The sketch commands: drawing entities, constraining them, and reading
//! back how the sketch solved.
//!
//! Coordinates are plain numbers in the document's length unit: they are
//! where an entity is drawn, and the solver moves it from there. Dimension
//! values are expressions. Entities are referred to as `"e3"` (or `3`),
//! constraints as `"c3"`.

use crate::commands::{Run, doc_error, edit, fail, feature_ref, feature_result, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    ConstraintId, ConstraintKind, DistanceDirection, Document, EntityId, Expr, FeatureId,
    FeatureKind, Geometry, PersistentName, Sketch, SketchError, SketchPlane, WorldPlane,
};
use craft3d_regen::{ConstraintState, SketchResult};
use serde_json::{Map, Value, json};

/// Most points one `sketch.polyline` takes.
const MAX_POLYLINE_POINTS: usize = 1_000;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("sketch.create", "New Sketch", r#"{"plane": "xy|yz|zx" or datum plane = "xy", "face": face name, "offset": expr = 0, "name": string = automatic}"#,
        "Adds an empty sketch on a world plane, on a datum plane (plane: its name or id; the sketch has the datum's origin and axes and follows it), or on a flat face of a body given by its name from document.inspect (faceNames, such as \"F2:endCap(e7,e8,e9)\"; \"F2:endCap()\" is the plane where F2 ends, whatever its outline; an alias from faceAliases names the same face), moved by offset along the normal. A primitive's faces are numbered (\"F3:face(1)\"): for a block 0 bottom, 1 top, 2 front, 3 back, 4 left, 5 right in its own axes; for a cylinder 0 the wall, 1 the bottom, 2 the top. On xy the sketch axes are world X and Y, on yz they are Y and Z, on zx they are Z and X. On the end cap of an extrusion they are those of the extruded sketch; on a side face x runs along its line and y along the extrusion, from the extruded sketch's origin dropped onto the face.", doc: true, sketch_create),
    spec!("sketch.point", "Sketch Point", r#"{"sketch": id or name, "at": [number, number]}"#,
        "Adds a point to a sketch, at coordinates in document units. Returns its entity id.", doc: true, sketch_point),
    spec!("sketch.line", "Sketch Line", r#"{"sketch": id or name, "from": point id or [x, y], "to": point id or [x, y], "construction": boolean = false}"#,
        "Adds a line between two points. Give an existing point (\"e1\") to share a corner, or coordinates to make a new point.", doc: true, sketch_line),
    spec!("sketch.polyline", "Sketch Polyline", r#"{"sketch": id or name, "points": [point id or [x, y], ...], "closed": boolean = false, "construction": boolean = false}"#,
        "Adds lines through the points in order, sharing the corners; closed joins the last point to the first. Returns the points and the lines.", doc: true, sketch_polyline),
    spec!("sketch.circle", "Sketch Circle", r#"{"sketch": id or name, "center": point id or [x, y], "radius": number, "construction": boolean = false}"#,
        "Adds a circle about a point.", doc: true, sketch_circle),
    spec!("sketch.arc", "Sketch Arc", r#"{"sketch": id or name, "center": point id or [x, y], "start": point id or [x, y], "end": point id or [x, y], "construction": boolean = false}"#,
        "Adds an arc about center, counter-clockwise from start to end. Solving keeps both ends at the same distance from the center.", doc: true, sketch_arc),
    spec!("sketch.constrain", "Constrain", r#"{"sketch": id or name, "type": "coincident|pointOn|horizontal|vertical|parallel|perpendicular|tangent|equal|concentric|symmetric|midpoint|fixed|distance|angle|radius|diameter", "entities": [entity id, ...], "value": expr, "direction": "direct|horizontal|vertical" = "direct"}"#,
        "Adds a constraint between entities. distance, angle, radius and diameter take a value; a distance takes one line (its length), two points, or a point and a line. symmetric takes two points and the mirror line. A spline is shaped by constraints on its control points; itself it takes only tangent, to a line, an arc or another spline that it shares an end point with (the same point, or two made coincident): the spline then leaves that point along the other curve. An angle is measured counter-clockwise from the first line to the second, each taken from its start to its end. Returns the constraint id and how the sketch solved.", doc: true, sketch_constrain),
    spec!("sketch.edit", "Edit Sketch", r#"{"sketch": id or name, "constraint": constraint id, "value": expr, "entity": entity id, "construction": boolean, "at": [number, number], "plane": "xy|yz|zx" or datum plane, "face": face name, "offset": expr}"#,
        "Changes a sketch: the value of a dimension (constraint and value), an entity's construction flag or a point's position before solving (entity with construction or at), or what it lies on (plane or face) and the offset.", doc: true, sketch_edit),
    spec!("sketch.delete", "Delete From Sketch", r#"{"sketch": id or name, "entity": entity id, "constraint": constraint id}"#,
        "Deletes a constraint, or an entity with the constraints on it. A point that lines, circles, arcs or splines are built on cannot be deleted before them.", doc: true, sketch_delete),
    spec!("sketch.inspect", "Inspect Sketch", r#"{"sketch": id or name}"#,
        "Returns a sketch in full: plane, entities with solved coordinates, constraints with their state, and the degrees of freedom left.", doc: true, sketch_inspect),
];

// ---------------------------------------------------------------- helpers

/// The sketch a command works on.
pub(crate) fn sketch_ref(
    session: &Session,
    p: &Params,
    command: &str,
) -> Result<FeatureId, CommandError> {
    let doc = session.document()?.history.current();
    let id = feature_ref(doc, p, command, "sketch")?;
    doc.sketch(id).map_err(doc_error(command))?;
    Ok(id)
}

/// An id written as a number or with its letter: `3`, `"e3"`.
fn local_id(command: &str, key: &str, prefix: char, value: &Value) -> Result<u64, CommandError> {
    let id = match value {
        Value::Number(n) => n.as_u64(),
        Value::String(text) => text
            .strip_prefix(prefix)
            .unwrap_or(text)
            .parse::<u64>()
            .ok(),
        _ => None,
    };
    id.ok_or_else(|| {
        let shown: String = value.to_string().chars().take(40).collect();
        CommandError::new(format!(
            "{command}: `{key}` must be an id such as \"{prefix}3\" or 3, got {shown}"
        ))
    })
}

fn entity_id(command: &str, key: &str, value: &Value) -> Result<EntityId, CommandError> {
    local_id(command, key, 'e', value).map(EntityId)
}

fn constraint_id(command: &str, key: &str, value: &Value) -> Result<ConstraintId, CommandError> {
    local_id(command, key, 'c', value).map(ConstraintId)
}

/// Two finite numbers.
pub(crate) fn coordinates(value: &Value) -> Option<(f64, f64)> {
    let [x, y] = value.as_array()?.as_slice() else {
        return None;
    };
    let finite = |v: &Value| v.as_f64().filter(|n| n.is_finite());
    Some((finite(x)?, finite(y)?))
}

/// Where a line, circle or arc is attached: a point that exists, or the
/// place for a new one.
#[derive(Clone, Copy)]
pub(crate) enum PointArg {
    Existing(EntityId),
    New(f64, f64),
}

impl PointArg {
    pub(crate) fn parse(command: &str, key: &str, value: &Value) -> Result<Self, CommandError> {
        match value {
            Value::Array(_) => match coordinates(value) {
                Some((x, y)) => Ok(PointArg::New(x, y)),
                None => fail(format!(
                    "{command}: `{key}` must be a point id or two finite numbers [x, y]"
                )),
            },
            Value::Number(_) | Value::String(_) => {
                entity_id(command, key, value).map(PointArg::Existing)
            }
            _ => fail(format!(
                "{command}: `{key}` must be a point id such as \"e1\" or coordinates [x, y]"
            )),
        }
    }

    pub(crate) fn required(p: &Params, command: &str, key: &str) -> Result<Self, CommandError> {
        Self::parse(command, key, p.get(key).ok_or_else(|| p.missing(key))?)
    }

    /// The point's id, adding it to the sketch when it is new.
    pub(crate) fn place(self, sketch: &mut Sketch) -> Result<EntityId, SketchError> {
        match self {
            PointArg::Existing(id) => sketch.point(id).map(|_| id),
            PointArg::New(x, y) => sketch.add_point(x, y),
        }
    }
}

/// The plane a sketch is put on by name: a base plane or a datum plane.
enum PlaneArg {
    World(WorldPlane),
    Datum(FeatureId),
}

impl PlaneArg {
    fn with(self, offset: Expr) -> SketchPlane {
        match self {
            PlaneArg::World(plane) => SketchPlane::World { plane, offset },
            PlaneArg::Datum(plane) => SketchPlane::Datum { plane, offset },
        }
    }
}

/// `"xy"`, `"yz"`, `"zx"`, or a datum plane by its name or id (`"F3"`).
fn plane_param(
    doc: &Document,
    p: &Params,
    command: &str,
) -> Result<Option<PlaneArg>, CommandError> {
    let Some(name) = p.str("plane")? else {
        return Ok(None);
    };
    if let Some(plane) = WorldPlane::from_name(name) {
        return Ok(Some(PlaneArg::World(plane)));
    }
    let datum = doc.feature_by_name(name).or_else(|| {
        let id = name.strip_prefix('F')?.parse().ok()?;
        doc.feature(FeatureId(id))
    });
    match datum {
        Some(feature) if matches!(feature.kind, FeatureKind::DatumPlane(_)) => {
            Ok(Some(PlaneArg::Datum(feature.id)))
        }
        Some(feature) => fail(format!(
            "{command}: `plane`: `{}` is a {}, not a datum plane; a sketch lies on xy, yz or zx, on a datum plane, or on a face (`face`)",
            feature.name,
            feature.kind.type_name()
        )),
        None => {
            let shown: String = name.chars().take(40).collect();
            fail(format!(
                "{command}: unknown plane `{shown}`; use xy, yz or zx, or the name of a datum plane"
            ))
        }
    }
}

/// The name of a face, as `document.inspect` lists it.
fn face_param(p: &Params, command: &str) -> Result<Option<PersistentName>, CommandError> {
    match p.str("face")? {
        None => Ok(None),
        Some(text) => text
            .parse::<PersistentName>()
            .map(Some)
            .map_err(|reason| CommandError::new(format!("{command}: `face`: {reason}"))),
    }
}

pub(crate) fn ids(list: &[EntityId]) -> Vec<String> {
    list.iter().map(|id| id.to_string()).collect()
}

fn state_name(state: &ConstraintState) -> &'static str {
    match state {
        ConstraintState::Satisfied => "satisfied",
        ConstraintState::Redundant => "redundant",
        ConstraintState::Conflicting => "conflicting",
        ConstraintState::Unsolved => "unsolved",
    }
}

/// How a sketch solved, as every sketch command reports it: the degrees of
/// freedom left and the constraints that are in the way.
fn solve_summary(map: &mut Map<String, Value>, result: &SketchResult) {
    map.insert("dof".into(), result.dof.map_or(Value::Null, Value::from));
    let listed = |wanted: ConstraintState| -> Vec<String> {
        result
            .constraints
            .iter()
            .filter(|c| c.state == wanted)
            .map(|c| c.id.to_string())
            .collect()
    };
    for (key, state) in [
        ("redundant", ConstraintState::Redundant),
        ("conflicting", ConstraintState::Conflicting),
    ] {
        let found = listed(state);
        if !found.is_empty() {
            map.insert(key.into(), found.into());
        }
    }
}

/// What a sketch command returns: the sketch feature, how it regenerated
/// and solved, and the command's own fields.
pub(crate) fn sketch_result(session: &mut Session, id: FeatureId, own: Value) -> Run {
    let mut result = feature_result(session, id)?;
    let regen = session.document_mut()?.regen();
    if let Some(map) = result.as_object_mut() {
        if let Some(solved) = regen.feature(id).and_then(|f| f.sketch.as_ref()) {
            solve_summary(map, solved);
        }
        if let Value::Object(own) = own {
            map.extend(own);
        }
    }
    Ok(result)
}

pub(crate) fn change<T>(
    session: &mut Session,
    command: &'static str,
    label: &str,
    id: FeatureId,
    change: impl FnOnce(&mut Sketch) -> Result<T, SketchError>,
) -> Result<T, CommandError> {
    edit(session, command, label, |doc: &mut Document| {
        doc.edit_sketch(id, change)
    })
}

// --------------------------------------------------------------- commands

fn sketch_create(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.create";
    let p = Params::new(COMMAND, params, &["plane", "face", "offset", "name"])?;
    let offset = Expr::new(p.expr("offset")?.unwrap_or_else(|| "0".to_string()));
    let doc = session.document()?.history.current();
    let plane = match (plane_param(doc, &p, COMMAND)?, face_param(&p, COMMAND)?) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{COMMAND}: give either a `plane` or a `face`, not both"
            ));
        }
        (plane, None) => plane
            .unwrap_or(PlaneArg::World(WorldPlane::default()))
            .with(offset),
        (None, Some(face)) => SketchPlane::Face { face, offset },
    };
    let name = p.str("name")?;
    let id = edit(session, COMMAND, "Add sketch", |doc| {
        doc.add_feature(name, FeatureKind::Sketch(Sketch::new(plane)))
    })?;
    sketch_result(session, id, Value::Null)
}

fn sketch_point(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.point";
    let p = Params::new(COMMAND, params, &["sketch", "at"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let at = p.get("at").ok_or_else(|| p.missing("at"))?;
    let Some((x, y)) = coordinates(at) else {
        return fail(format!("{COMMAND}: `at` must be two finite numbers [x, y]"));
    };
    let point = change(session, COMMAND, "Add point", id, |s| s.add_point(x, y))?;
    sketch_result(session, id, json!({ "entity": point.to_string() }))
}

fn sketch_line(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.line";
    let p = Params::new(COMMAND, params, &["sketch", "from", "to", "construction"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let from = PointArg::required(&p, COMMAND, "from")?;
    let to = PointArg::required(&p, COMMAND, "to")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let (line, start, end) = change(session, COMMAND, "Add line", id, |s| {
        let (start, end) = (from.place(s)?, to.place(s)?);
        Ok((s.add_line(start, end, construction)?, start, end))
    })?;
    sketch_result(
        session,
        id,
        json!({ "entity": line.to_string(), "start": start.to_string(), "end": end.to_string() }),
    )
}

fn sketch_polyline(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.polyline";
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "points", "closed", "construction"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let Some(Value::Array(items)) = p.get("points") else {
        return match p.get("points") {
            None => Err(p.missing("points")),
            Some(_) => fail(format!(
                "{COMMAND}: `points` must be an array of point ids or coordinates [x, y]"
            )),
        };
    };
    if items.len() > MAX_POLYLINE_POINTS {
        return fail(format!(
            "{COMMAND}: at most {MAX_POLYLINE_POINTS} points at a time, got {}",
            items.len()
        ));
    }
    let closed = p.bool("closed")?.unwrap_or(false);
    let construction = p.bool("construction")?.unwrap_or(false);
    let needed = if closed { 3 } else { 2 };
    if items.len() < needed {
        return fail(format!(
            "{COMMAND}: {} polyline needs at least {needed} points, got {}",
            if closed { "a closed" } else { "a" },
            items.len()
        ));
    }
    let mut corners = Vec::with_capacity(items.len());
    for item in items {
        corners.push(PointArg::parse(COMMAND, "points", item)?);
    }
    let (points, lines) = change(session, COMMAND, "Add polyline", id, |s| {
        let mut points = Vec::with_capacity(corners.len());
        for corner in &corners {
            points.push(corner.place(s)?);
        }
        let mut lines = Vec::with_capacity(points.len());
        for pair in points.windows(2) {
            if let [a, b] = pair {
                lines.push(s.add_line(*a, *b, construction)?);
            }
        }
        if let (true, Some(first), Some(last)) = (closed, points.first(), points.last()) {
            lines.push(s.add_line(*last, *first, construction)?);
        }
        Ok((points, lines))
    })?;
    sketch_result(
        session,
        id,
        json!({ "points": ids(&points), "lines": ids(&lines) }),
    )
}

fn sketch_circle(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.circle";
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "center", "radius", "construction"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let center = PointArg::required(&p, COMMAND, "center")?;
    let radius = p.number("radius")?.ok_or_else(|| p.missing("radius"))?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let (circle, center) = change(session, COMMAND, "Add circle", id, |s| {
        let center = center.place(s)?;
        Ok((s.add_circle(center, radius, construction)?, center))
    })?;
    sketch_result(
        session,
        id,
        json!({ "entity": circle.to_string(), "center": center.to_string() }),
    )
}

fn sketch_arc(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.arc";
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "center", "start", "end", "construction"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let center = PointArg::required(&p, COMMAND, "center")?;
    let start = PointArg::required(&p, COMMAND, "start")?;
    let end = PointArg::required(&p, COMMAND, "end")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let (arc, points) = change(session, COMMAND, "Add arc", id, |s| {
        let points = [center.place(s)?, start.place(s)?, end.place(s)?];
        let [c, a, b] = points;
        Ok((s.add_arc(c, a, b, construction)?, points))
    })?;
    let [center, start, end] = points;
    sketch_result(
        session,
        id,
        json!({
            "entity": arc.to_string(), "center": center.to_string(),
            "start": start.to_string(), "end": end.to_string(),
        }),
    )
}

fn sketch_constrain(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.constrain";
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "type", "entities", "value", "direction"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let type_name = p.required_str("type")?;
    let Some(Value::Array(items)) = p.get("entities") else {
        return match p.get("entities") {
            None => Err(p.missing("entities")),
            Some(_) => fail(format!(
                "{COMMAND}: `entities` must be an array of entity ids, such as [\"e1\", \"e2\"]"
            )),
        };
    };
    // No constraint takes more than three.
    if items.len() > 3 {
        return fail(format!(
            "{COMMAND}: a constraint takes at most three entities, got {}",
            items.len()
        ));
    }
    let mut entities = Vec::with_capacity(items.len());
    for item in items {
        entities.push(entity_id(COMMAND, "entities", item)?);
    }
    let direction = match p.str("direction")? {
        None | Some("direct") => DistanceDirection::Direct,
        Some("horizontal") => DistanceDirection::Horizontal,
        Some("vertical") => DistanceDirection::Vertical,
        Some(other) => {
            let shown: String = other.chars().take(40).collect();
            return fail(format!(
                "{COMMAND}: unknown direction `{shown}`; use direct, horizontal or vertical"
            ));
        }
    };
    let value = p.expr("value")?.map(Expr::new);
    let kind =
        ConstraintKind::from_parts(type_name, &entities, value, direction).map_err(|reason| {
            let shown: String = reason.chars().take(400).collect();
            CommandError::new(format!("{COMMAND}: {shown}"))
        })?;
    let label = format!("Constrain {}", kind.type_name());
    let constraint = change(session, COMMAND, &label, id, |s| s.add_constraint(kind))?;
    sketch_result(session, id, json!({ "constraint": constraint.to_string() }))
}

fn sketch_edit(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.edit";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "constraint",
            "value",
            "entity",
            "construction",
            "at",
            "plane",
            "face",
            "offset",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let constraint = p
        .get("constraint")
        .map(|v| constraint_id(COMMAND, "constraint", v))
        .transpose()?;
    let value = p.expr("value")?.map(Expr::new);
    let entity = p
        .get("entity")
        .map(|v| entity_id(COMMAND, "entity", v))
        .transpose()?;
    let construction = p.bool("construction")?;
    let at = match p.get("at") {
        None => None,
        Some(v) => Some(coordinates(v).ok_or_else(|| {
            CommandError::new(format!("{COMMAND}: `at` must be two finite numbers [x, y]"))
        })?),
    };
    let plane = plane_param(session.document()?.history.current(), &p, COMMAND)?;
    let face = face_param(&p, COMMAND)?;
    if plane.is_some() && face.is_some() {
        return fail(format!(
            "{COMMAND}: give either a `plane` or a `face`, not both"
        ));
    }
    let offset = p.expr("offset")?.map(Expr::new);

    match (&constraint, &value) {
        (Some(_), None) => return fail(format!("{COMMAND}: `constraint` needs a `value`")),
        (None, Some(_)) => return fail(format!("{COMMAND}: `value` needs a `constraint`")),
        _ => {}
    }
    match (&entity, construction.is_some() || at.is_some()) {
        (Some(_), false) => {
            return fail(format!("{COMMAND}: `entity` needs `construction` or `at`"));
        }
        (None, true) => {
            return fail(format!(
                "{COMMAND}: `construction` and `at` need an `entity`"
            ));
        }
        _ => {}
    }
    if constraint.is_none()
        && entity.is_none()
        && plane.is_none()
        && face.is_none()
        && offset.is_none()
    {
        return fail(format!(
            "{COMMAND}: give something to change: constraint and value, entity and construction or at, plane or face, or offset"
        ));
    }
    change(session, COMMAND, "Edit sketch", id, |s| {
        if let (Some(constraint), Some(value)) = (constraint, value) {
            let kind = &mut s.constraint_mut(constraint)?.kind;
            let type_name = kind.type_name();
            match kind.value_mut() {
                Some(slot) => *slot = value,
                None => {
                    return Err(SketchError::Invalid(format!(
                        "{constraint} is a {type_name} constraint: it has no value"
                    )));
                }
            }
        }
        if let Some(entity) = entity {
            let target = s.entity_mut(entity)?;
            if let Some(construction) = construction {
                target.construction = construction;
            }
            if let Some((x, y)) = at {
                match &mut target.geometry {
                    Geometry::Point { x: px, y: py } => (*px, *py) = (x, y),
                    other => {
                        return Err(SketchError::Invalid(format!(
                            "{entity} is a {}: only a point can be moved with `at`",
                            other.type_name()
                        )));
                    }
                }
            }
        }
        // A new plane or face keeps the offset unless one is given too.
        let kept = s.plane.offset().clone();
        if let Some(plane) = plane {
            s.plane = plane.with(kept);
        } else if let Some(face) = face {
            s.plane = SketchPlane::Face { face, offset: kept };
        }
        if let Some(offset) = offset {
            *s.plane.offset_mut() = offset;
        }
        Ok(())
    })?;
    sketch_result(session, id, Value::Null)
}

fn sketch_delete(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.delete";
    let p = Params::new(COMMAND, params, &["sketch", "entity", "constraint"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let entity = p
        .get("entity")
        .map(|v| entity_id(COMMAND, "entity", v))
        .transpose()?;
    let constraint = p
        .get("constraint")
        .map(|v| constraint_id(COMMAND, "constraint", v))
        .transpose()?;
    let own = match (entity, constraint) {
        (Some(entity), None) => {
            let gone = change(session, COMMAND, "Delete entity", id, |s| {
                s.remove_entity(entity)
            })?;
            let gone: Vec<String> = gone.iter().map(|c| c.to_string()).collect();
            json!({ "deleted": entity.to_string(), "constraintsDeleted": gone })
        }
        (None, Some(constraint)) => {
            change(session, COMMAND, "Delete constraint", id, |s| {
                s.remove_constraint(constraint)
            })?;
            json!({ "deleted": constraint.to_string() })
        }
        _ => {
            return fail(format!(
                "{COMMAND}: give either an `entity` or a `constraint` to delete"
            ));
        }
    };
    sketch_result(session, id, own)
}

fn sketch_inspect(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.inspect";
    let p = Params::new(COMMAND, params, &["sketch"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let mut result = feature_result(session, id)?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let sketch = doc.sketch(id).map_err(doc_error(COMMAND))?;
    let solved = regen.feature(id).and_then(|f| f.sketch.as_ref());

    let entities: Vec<Value> = sketch
        .entities
        .iter()
        .map(|entity| {
            let mut entry = json!({
                "id": entity.id.to_string(),
                "type": entity.geometry.type_name(),
                "construction": entity.construction,
            });
            if let Some(map) = entry.as_object_mut() {
                match entity.geometry {
                    Geometry::Spline {
                        degree,
                        ref control,
                    } => {
                        map.insert("degree".into(), degree.into());
                        map.insert("control".into(), ids(control).into());
                        map.insert(
                            "closed".into(),
                            (control.len() > 1 && control.first() == control.last()).into(),
                        );
                    }
                    Geometry::Point { x, y } => {
                        // The solution of this regeneration when there is
                        // one; what is stored otherwise.
                        let (x, y) = solved.and_then(|s| s.point(entity.id)).unwrap_or((x, y));
                        map.insert("x".into(), x.into());
                        map.insert("y".into(), y.into());
                    }
                    Geometry::Line { start, end } => {
                        map.insert("start".into(), start.to_string().into());
                        map.insert("end".into(), end.to_string().into());
                    }
                    Geometry::Circle { center, radius } => {
                        let radius = solved.and_then(|s| s.radius(entity.id)).unwrap_or(radius);
                        map.insert("center".into(), center.to_string().into());
                        map.insert("radius".into(), radius.into());
                    }
                    Geometry::Arc { center, start, end } => {
                        map.insert("center".into(), center.to_string().into());
                        map.insert("start".into(), start.to_string().into());
                        map.insert("end".into(), end.to_string().into());
                    }
                }
            }
            entry
        })
        .collect();
    let constraints: Vec<Value> = sketch
        .constraints
        .iter()
        .map(|constraint| {
            let kind = &constraint.kind;
            let mut entry = json!({
                "id": constraint.id.to_string(),
                "type": kind.type_name(),
                "entities": ids(&kind.entities()),
            });
            if let Some(map) = entry.as_object_mut() {
                if let Some(value) = kind.value() {
                    map.insert("value".into(), value.source().into());
                }
                if let ConstraintKind::Distance { direction, .. } = kind
                    && *direction != DistanceDirection::Direct
                {
                    let name = match direction {
                        DistanceDirection::Direct => "direct",
                        DistanceDirection::Horizontal => "horizontal",
                        DistanceDirection::Vertical => "vertical",
                    };
                    map.insert("direction".into(), name.into());
                }
                if let Some(outcome) =
                    solved.and_then(|s| s.constraints.iter().find(|c| c.id == constraint.id))
                {
                    map.insert("state".into(), state_name(&outcome.state).into());
                    if let Some(residual) = outcome.residual {
                        map.insert("residual".into(), residual.into());
                    }
                }
            }
            entry
        })
        .collect();
    if let Some(map) = result.as_object_mut() {
        if let Some(solved) = solved {
            solve_summary(map, solved);
            map.insert(
                "frame".into(),
                json!({
                    "origin": [solved.origin.x, solved.origin.y, solved.origin.z],
                    "x": [solved.x_axis.x, solved.x_axis.y, solved.x_axis.z],
                    "y": [solved.y_axis.x, solved.y_axis.y, solved.y_axis.z],
                }),
            );
        }
        map.insert(
            "plane".into(),
            serde_json::to_value(&sketch.plane).unwrap_or(Value::Null),
        );
        map.insert("entities".into(), entities.into());
        map.insert("constraints".into(), constraints.into());
        map.insert("lengthUnit".into(), doc.units.length.symbol().into());
    }
    Ok(result)
}
