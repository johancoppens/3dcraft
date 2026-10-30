//! The sketch commands: drawing entities, constraining them, and reading
//! back how the sketch solved.
//!
//! A coordinate given as a number, in the document's length unit, is where
//! a point is drawn, and the solver moves it from there. A coordinate given
//! as an expression (a string: `"-width / 2"`, `"10 mm"`) is drawn at its
//! value and held there by an `x` or `y` constraint, so it follows the
//! variables. Dimension values are expressions. Entities are referred to
//! as `"e3"` (or `3`), constraints as `"c3"`.

use crate::commands::{
    Run, add_failed, doc_error, edit, fail, failing, feature_ref, feature_result, insert_feature,
    spec,
};
use crate::params::Params;
use crate::typed::Typed;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    ConstraintId, ConstraintKind, DistanceDirection, Document, EntityId, Expr, FeatureId,
    FeatureKind, Geometry, InstanceIndex, MAX_PATTERN_DEPTH, PersistentName, Removal, Sketch,
    SketchError, SketchPlane, WorldPlane, parse_image_path,
};
use craft3d_regen::{ConstraintState, SketchResult};
use serde_json::{Map, Value, json};

/// Most points one `sketch.polyline` takes.
const MAX_POLYLINE_POINTS: usize = 1_000;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("sketch.create", "New Sketch", r#"{"plane": "xy|yz|zx" or datum plane = "xy", "face": face name, "offset": expr = 0, "xAxis": [number, number, number] = the face's own, "name": string = automatic}"#,
        "Adds an empty sketch on a world plane, on a datum plane (plane: its name or id; the sketch has the datum's origin and axes and follows it), or on a flat face of a body given by its name from document.inspect (faceNames, such as \"F2:endCap(e7,e8,e9)\"; \"F2:endCap()\" is the plane where F2 ends, whatever its outline; an alias from faceAliases names the same face), moved by offset along the normal. A primitive's faces are numbered (\"F3:face(1)\"): for a block 0 bottom, 1 top, 2 front, 3 back, 4 left, 5 right in its own axes; for a cylinder 0 the wall, 1 the bottom, 2 the top. On xy the sketch axes are world X and Y, on yz they are Y and Z, on zx they are Z and X. On the end cap of an extrusion they are those of the extruded sketch; on a side face x runs along its line and y along the extrusion, from the extruded sketch's origin dropped onto the face. xAxis (a face only) turns the sketch's x axis to a world direction projected onto the face (three plain numbers, any length; [1, 0, 0] puts x along world X on the bottom of a block, whose own x is along world Y), about the face's origin, with y the normal × x; the normal stays the face's. A direction along the normal fails the sketch. Without it the axes are the face's own.", doc: true, names: ["face", "plane"], sketch_create),
    spec!("sketch.point", "Sketch Point", r#"{"sketch": id or name, "at": [x, y]}"#,
        "Adds a point to a sketch. Coordinates [x, y] that are both numbers, in document units, are where the point is drawn; the solver may move it. When either is an expression (a string such as \"-width / 2\" or \"10 mm\"; signed, from the sketch origin along the sketch axes) the point is held: drawn at the values and kept there by an x and a y constraint (a number beside an expression is held at that number), so it follows the variables; to hold one coordinate only, use sketch.constrain with type x or y. Returns its entity id, held (true for a held point, false for a free one the solver may move: [0, 0] is free, [0, \"0 mm\"] is held) and the constraints that hold it. Everywhere a sketch command takes coordinates [x, y] for a new point (line, polyline, rect, circle, arc, path, spline) they work the same way.", doc: true, sketch_point),
    spec!("sketch.line", "Sketch Line", r#"{"sketch": id or name, "from": point id or [x, y], "to": point id or [x, y], "construction": boolean = false}"#,
        "Adds a line between two points. Give an existing point (\"e1\") to share a corner, or coordinates to make a new point.", doc: true, sketch_line),
    spec!("sketch.polyline", "Sketch Polyline", r#"{"sketch": id or name, "points": [point id or [x, y], ...], "closed": boolean = false, "construction": boolean = false}"#,
        "Adds lines through the points in order, sharing the corners; closed joins the last point to the first. Returns the points and the lines.", doc: true, sketch_polyline),
    spec!("sketch.rect", "Sketch Rectangle", r#"{"sketch": id or name, "from": point id or [x, y], "to": point id or [x, y], "construction": boolean = false}"#,
        "Adds a rectangle along the sketch axes with opposite corners from and to (coordinates may be expressions, held as for sketch.point: [\"-width / 2\", 0] to [\"width / 2\", \"height\"] is fully defined and follows the variables). Four lines, kept horizontal and vertical by constraints. Returns points (from, then the corner at to's x and from's y, to, the corner at from's x and to's y), lines (from points[i] to points[i+1], the last back to from) and constraints.", doc: true, sketch_rect),
    spec!("sketch.circle", "Sketch Circle", r#"{"sketch": id or name, "center": point id or [x, y], "radius": number or expr, "construction": boolean = false}"#,
        "Adds a circle about a point. A radius given as a number is how it is drawn; one given as an expression (a string such as \"bore / 2\") is held by a radius constraint, so it follows the variables. Returns the circle, its center and constraints: the radius constraint, when the radius is an expression (those that hold a center given by expressions are listed by sketch.inspect).", doc: true, sketch_circle),
    spec!("sketch.arc", "Sketch Arc", r#"{"sketch": id or name, "center": point id or [x, y], "start": point id or [x, y], "end": point id or [x, y], "construction": boolean = false}"#,
        "Adds an arc about center, counter-clockwise from start to end. Solving keeps both ends at the same distance from the center.", doc: true, sketch_arc),
    spec!("sketch.constrain", "Constrain", r#"{"sketch": id or name, "type": "coincident|pointOn|horizontal|vertical|parallel|perpendicular|tangent|equal|concentric|symmetric|midpoint|fixed|x|y|distance|angle|radius|diameter", "entities": [entity id, ...], "value": expr, "direction": "direct|horizontal|vertical" = "direct"}"#,
        "Adds a constraint between entities. x, y, distance, angle, radius and diameter take a value; x and y hold one point's coordinate in the sketch, signed (\"-width / 2\" is left of the origin); a distance takes one line (its length), two points, or a point and a line. symmetric takes two points and the mirror line. A spline is shaped by constraints on its control points; itself it takes pointOn (a point held on the curve: one degree of freedom less; beyond its ends the curve counts as going on straight) and tangent, to a line, an arc or another spline that it shares an end point with (the same point, or two made coincident): the spline then leaves that point along the other curve. A formula curve (sketch.curve) and a live spline (sketch.spline live) take pointOn and tangent too: tangent at an end point shared with a line, an arc, a spline or each other, or, without a shared end, to a line or a circle or arc that touches the curve anywhere (a line where the curve runs parallel to it, the place nearest the line; a circle at the foot of its centre, on the side the centre starts on); two formula curves take no tangent. An angle is measured counter-clockwise from the first line to the second, each taken from its start to its end. Returns the constraint id and how the sketch solved.", doc: true, sketch_constrain),
    spec!("sketch.edit", "Edit Sketch", r#"{"sketch": id or name, "constraint": constraint id, "value": expr, "entity": entity id, "construction": boolean, "at": [number, number], "plane": "xy|yz|zx" or datum plane, "face": face name, "offset": expr}"#,
        "Changes a sketch: the value of a dimension (constraint and value), an entity's construction flag or a point's position before solving (entity with construction or at), or what it lies on (plane or face) and the offset.", doc: true, names: ["face", "plane"], sketch_edit),
    spec!("sketch.delete", "Delete From Sketch", r#"{"sketch": id or name, "entity": entity id, "constraint": constraint id, "pattern": pattern id}"#,
        "Deletes a constraint, a pattern (\"p1\": with its image points and the constraints on them), or an entity with the constraints on it. A point that lines, circles, arcs or splines are built on cannot be deleted before them. An entity of a pattern's seed leaves the seed; a pattern left without seed, or without a point its chain or centre uses, is deleted too, and image points of what went go with their constraints; the result lists all of it.", doc: true, sketch_delete),
    spec!("sketch.inspect", "Inspect Sketch", r#"{"sketch": id or name}"#,
        "Returns a sketch in full: plane, entities with solved coordinates (an image point with the seed point, pattern and instance it stands for: \"e5[last]\" with the instance it is now), constraints with their state (missing: it uses an image point of an instance its pattern no longer has), the degrees of freedom left, and the patterns: seed, count and instances as evaluated, layout, whether a ring closes and how its chain solved, and the names of the curves it makes (e7[3], e7[2,4]; a cap whose loop holds a whole run is named with e7[*]).", doc: true, sketch_inspect),
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
pub(crate) fn local_id(
    command: &str,
    key: &str,
    prefix: char,
    value: &Value,
) -> Result<u64, CommandError> {
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

pub(crate) fn entity_id(command: &str, key: &str, value: &Value) -> Result<EntityId, CommandError> {
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

/// The coordinates of a new point that were written as expressions: each
/// becomes an `x` or `y` constraint on the point, so the point follows the
/// variables the expression names.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Held {
    pub(crate) x: Option<Expr>,
    pub(crate) y: Option<Expr>,
}

impl Held {
    /// Adds the constraints that hold `point` at the expressions, `x`
    /// first; returns their ids.
    pub(crate) fn pin(
        &self,
        sketch: &mut Sketch,
        point: EntityId,
    ) -> Result<Vec<ConstraintId>, SketchError> {
        let mut added = Vec::new();
        if let Some(value) = &self.x {
            added.push(sketch.add_constraint(ConstraintKind::X {
                point,
                value: value.clone(),
            })?);
        }
        if let Some(value) = &self.y {
            added.push(sketch.add_constraint(ConstraintKind::Y {
                point,
                value: value.clone(),
            })?);
        }
        Ok(added)
    }
}

/// A finite number as the shortest expression that reads back to exactly
/// it: the plain decimal (`12.5`) or, when that is longer, the scientific
/// form (`1e300`, `2.5e-7`). Both are Rust's shortest round-tripping
/// digits.
pub(crate) fn number_expr(v: f64) -> String {
    let plain = v.to_string();
    let scientific = format!("{v:e}");
    if scientific.len() < plain.len() {
        scientific
    } else {
        plain
    }
}

/// Most characters of a coordinate written as an expression.
pub(crate) const MAX_COORDINATE_EXPR: usize = 1_000;

/// `[x, y]` for a new point: two numbers (where it is drawn), or a pair
/// with an expression in it (a string, evaluated with the document's
/// variables): drawn there and held there, a number beside the expression
/// at that number. `None` when `value` is not such a pair.
pub(crate) fn held_coordinates(
    typed: &Typed<'_>,
    command: &str,
    key: &str,
    value: &Value,
) -> Result<Option<(f64, f64, Held)>, CommandError> {
    let Some([x, y]) = value.as_array().map(Vec::as_slice) else {
        return Ok(None);
    };
    let one = |axis: &str, item: &Value| -> Result<Option<(f64, Option<Expr>)>, CommandError> {
        match item {
            Value::Number(n) => Ok(n.as_f64().filter(|v| v.is_finite()).map(|v| (v, None))),
            Value::String(text) => {
                if text.chars().count() > MAX_COORDINATE_EXPR {
                    return fail(format!(
                        "{command}: `{key}`: the {axis} expression is longer than {MAX_COORDINATE_EXPR} characters"
                    ));
                }
                let at = typed
                    .evaluate(text, crate::typed::TypedKind::Length)
                    .map_err(|e| {
                        CommandError::new(format!("{command}: `{key}`: {axis}: {}", e.message()))
                    })?;
                Ok(Some((at, Some(Expr::new(text.clone())))))
            }
            _ => Ok(None),
        }
    };
    match (one("x", x)?, one("y", y)?) {
        // A pair with an expression in it is held whole: a number beside
        // the expression is held at that number (`["-width / 2", 0]` is a
        // place, not a guess along y).
        (Some((x, held_x)), Some((y, held_y))) if held_x.is_some() || held_y.is_some() => {
            let number = |v: f64| Expr::new(number_expr(v));
            Ok(Some((
                x,
                y,
                Held {
                    x: Some(held_x.unwrap_or_else(|| number(x))),
                    y: Some(held_y.unwrap_or_else(|| number(y))),
                },
            )))
        }
        (Some((x, _)), Some((y, _))) => Ok(Some((x, y, Held::default()))),
        _ => Ok(None),
    }
}

/// The instances of an image point, one per level of its patterns, the
/// innermost first (`e8[last][2]`): at most `MAX_PATTERN_DEPTH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImagePath {
    levels: [InstanceIndex; MAX_PATTERN_DEPTH],
    len: usize,
}

impl ImagePath {
    fn new(levels: &[InstanceIndex]) -> Option<Self> {
        let mut out = ImagePath {
            levels: [InstanceIndex::At(0); MAX_PATTERN_DEPTH],
            len: 0,
        };
        for (slot, level) in out.levels.iter_mut().zip(levels) {
            *slot = *level;
        }
        out.len = levels.len();
        (1..=MAX_PATTERN_DEPTH)
            .contains(&levels.len())
            .then_some(out)
    }

    pub(crate) fn levels(&self) -> &[InstanceIndex] {
        self.levels.get(..self.len).unwrap_or_default()
    }
}

impl std::fmt::Display for ImagePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, level) in self.levels().iter().enumerate() {
            if i == 0 {
                write!(f, "{level}")?;
            } else {
                write!(f, "][{level}")?;
            }
        }
        Ok(())
    }
}

/// Where a line, circle or arc is attached: a point that exists, or the
/// place for a new one, with the coordinates written as expressions.
#[derive(Clone)]
pub(crate) enum PointArg {
    Existing(EntityId),
    New(f64, f64, Held),
    /// The image of a seed point in an instance of its pattern: `"e5[3]"`,
    /// `"e5[last]"`, and with nested patterns `"e5[last][2]"`.
    Image(EntityId, ImagePath),
}

/// An entity as a command names it: an id, or an image point of a pattern
/// (`"e5[3]"`, `"e5[2,4]"`, `"e5[last]"`, `"e5[last][2]"` for nested
/// patterns), made when it is not there yet.
#[derive(Clone, Copy)]
pub(crate) enum EntityArg {
    Id(EntityId),
    Image(EntityId, ImagePath),
}

impl EntityArg {
    pub(crate) fn parse(command: &str, key: &str, value: &Value) -> Result<Self, CommandError> {
        if let Value::String(text) = value
            && let Some(image) = parse_image_path(text)
        {
            let (of, levels) = image
                .map_err(|reason| CommandError::new(format!("{command}: `{key}`: {reason}")))?;
            let path = ImagePath::new(&levels).ok_or_else(|| {
                CommandError::new(format!(
                    "{command}: `{key}`: an image point has an instance for each of 1 to {MAX_PATTERN_DEPTH} levels of patterns"
                ))
            })?;
            return Ok(EntityArg::Image(of, path));
        }
        entity_id(command, key, value).map(EntityArg::Id)
    }

    /// The entity's id, adding the image points when they are new.
    pub(crate) fn place(self, sketch: &mut Sketch) -> Result<EntityId, SketchError> {
        match self {
            EntityArg::Id(id) => Ok(id),
            EntityArg::Image(of, path) => sketch.image_path(of, path.levels()),
        }
    }

    /// For an image point with a fixed instance on a level (`e5[3]`,
    /// `e5[last][2]`): refused when that level's pattern, as the last
    /// regeneration evaluated it, has no such instance (past the count or
    /// skipped). An image that is not there would only fail the sketch.
    pub(crate) fn check_instance(
        self,
        session: &mut Session,
        sketch: FeatureId,
        command: &str,
    ) -> Result<(), CommandError> {
        let EntityArg::Image(of, path) = self else {
            return Ok(());
        };
        let open = session.document_mut()?;
        let regen = open.regen();
        let doc = open.history.current();
        let Ok(drawn) = doc.sketch(sketch) else {
            return Ok(());
        };
        let Some(solved) = regen.feature(sketch).and_then(|f| f.sketch.as_ref()) else {
            return Ok(());
        };
        // The levels the path walks; one the sketch refuses (an unknown
        // point, an index of the wrong kind) is refused there with its
        // reason.
        let Some(owner) = drawn.image_owner(of) else {
            return Ok(());
        };
        for (index, pattern) in path.levels().iter().zip(drawn.pattern_levels(owner)) {
            if *index == InstanceIndex::Last {
                continue;
            }
            let fits = drawn
                .pattern(pattern)
                .is_some_and(|p| matches!(index, InstanceIndex::Grid(_)) == p.layout.is_grid());
            if !fits {
                continue;
            }
            if let Some(result) = solved.patterns.iter().find(|r| r.id == pattern)
                && result.resolve(*index).is_none()
            {
                return fail(format!(
                    "{command}: {of}[{path}] is not there: {}",
                    result.describe()
                ));
            }
        }
        Ok(())
    }
}

impl PointArg {
    pub(crate) fn parse(
        typed: &Typed<'_>,
        command: &str,
        key: &str,
        value: &Value,
    ) -> Result<Self, CommandError> {
        match value {
            Value::Array(_) => match held_coordinates(typed, command, key, value)? {
                Some((x, y, held)) => Ok(PointArg::New(x, y, held)),
                None => fail(format!(
                    "{command}: `{key}` must be a point id or coordinates [x, y]: two finite numbers, or expressions such as \"-width / 2\""
                )),
            },
            Value::Number(_) | Value::String(_) => match EntityArg::parse(command, key, value)? {
                EntityArg::Id(id) => Ok(PointArg::Existing(id)),
                EntityArg::Image(of, path) => Ok(PointArg::Image(of, path)),
            },
            _ => fail(format!(
                "{command}: `{key}` must be a point id such as \"e1\" or coordinates [x, y]"
            )),
        }
    }

    pub(crate) fn required(
        typed: &Typed<'_>,
        p: &Params,
        command: &str,
        key: &str,
    ) -> Result<Self, CommandError> {
        Self::parse(
            typed,
            command,
            key,
            p.get(key).ok_or_else(|| p.missing(key))?,
        )
    }

    /// The point's id, adding it to the sketch when it is new (with the
    /// constraints that hold its expressions).
    pub(crate) fn place(&self, sketch: &mut Sketch) -> Result<EntityId, SketchError> {
        match self {
            PointArg::Existing(id) => sketch.point_entity(*id).map(|_| *id),
            PointArg::New(x, y, held) => {
                let point = sketch.add_point(*x, *y)?;
                held.pin(sketch, point)?;
                Ok(point)
            }
            PointArg::Image(of, path) => sketch.image_path(*of, path.levels()),
        }
    }

    /// Where the point is drawn: a new one's coordinates, an existing
    /// one's stored position; `None` for an image point.
    pub(crate) fn drawn(&self, sketch: &Sketch) -> Option<(f64, f64)> {
        match self {
            PointArg::Existing(id) => sketch.point(*id).ok(),
            PointArg::New(x, y, _) => Some((*x, *y)),
            PointArg::Image(..) => None,
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
/// `xAxis`: a world direction, three plain numbers or expressions without
/// a unit; checked to evaluate when the sketch is placed.
fn x_axis_param(p: &Params, command: &str) -> Result<Option<[Expr; 3]>, CommandError> {
    let Some(value) = p.get("xAxis") else {
        return Ok(None);
    };
    let items = value.as_array().filter(|a| a.len() == 3).ok_or_else(|| {
        CommandError::new(format!(
            "{command}: `xAxis` is a direction: three numbers [x, y, z] in world coordinates"
        ))
    })?;
    let mut axis: [Expr; 3] = [Expr::new("0"), Expr::new("0"), Expr::new("0")];
    for (slot, item) in axis.iter_mut().zip(items) {
        *slot = match item {
            Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => Expr::new(n.to_string()),
            Value::String(text) if !text.trim().is_empty() && text.len() <= 256 => {
                Expr::new(text.clone())
            }
            _ => {
                return fail(format!(
                    "{command}: `xAxis` is a direction: three numbers [x, y, z] in world coordinates"
                ));
            }
        };
    }
    Ok(Some(axis))
}

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
        ConstraintState::Missing => "missing",
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
/// and solved, and the command's own fields. Given the features that
/// failed `before` the edit ([`failing`], taken before it), also
/// `failed`: the features made from the sketch that the edit made fail
/// (the sketch itself carries its own status).
pub(crate) fn sketch_result(
    session: &mut Session,
    id: FeatureId,
    before: Option<&[FeatureId]>,
    own: Value,
) -> Run {
    let mut result = feature_result(session, id)?;
    if let Some(before) = before {
        add_failed(session, before, Some(id), &mut result)?;
    }
    let regen = session.document_mut()?.regen();
    if let Some(map) = result.as_object_mut() {
        if let Some(solved) = regen.feature(id).and_then(|f| f.sketch.as_ref()) {
            solve_summary(map, solved);
            // Which constraints each conflicting one contradicts.
            let conflicting: Vec<_> = solved
                .constraints
                .iter()
                .filter(|c| c.state == ConstraintState::Conflicting)
                .map(|c| c.id)
                .collect();
            if !conflicting.is_empty() {
                let found =
                    crate::sketch_conflicts::contradictions(session.document()?, id, &conflicting);
                if !found.entries.is_empty() {
                    map.insert("conflicts".into(), found.entries.into());
                }
                if found.truncated {
                    map.insert("conflictsTruncated".into(), true.into());
                }
            }
            // A line shrunk to a point under directions: a contradiction
            // the solver reports as holding.
            let doc = session.document()?.history.current();
            if let Ok(sketch) = doc.sketch(id) {
                let collapsed =
                    crate::sketch_warnings::collapsed_lines(sketch, solved, doc.tolerance.linear);
                if !collapsed.is_empty() {
                    map.insert("collapsed".into(), collapsed.into());
                }
            }
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
    let p = Params::new(
        COMMAND,
        params,
        &["plane", "face", "offset", "xAxis", "name"],
    )?;
    let offset = Expr::new(p.expr("offset")?.unwrap_or_else(|| "0".to_string()));
    let x_axis = x_axis_param(&p, COMMAND)?;
    if x_axis.is_some() && !p.has("face") {
        return fail(format!(
            "{COMMAND}: `xAxis` turns the axes of a sketch on a face; a world or datum plane has axes of its own"
        ));
    }
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
        (None, Some(face)) => SketchPlane::Face {
            face,
            offset,
            x_axis,
        },
    };
    let name = p.str("name")?;
    let id = edit(session, COMMAND, "Add sketch", |doc| {
        insert_feature(doc, name, FeatureKind::Sketch(Sketch::new(plane)))
    })?;
    // Where the sketch lies, in world coordinates: on a face its axes may
    // be any of the world's, turned or swapped (the bottom of a block has
    // x along world Y), so an agent reads them before placing anything.
    let frame = session
        .document_mut()?
        .regen()
        .feature(id)
        .and_then(|f| f.sketch.as_deref())
        .map(frame_json);
    sketch_result(session, id, None, frame.unwrap_or(Value::Null))
}

/// A solved sketch's place in world coordinates, as keys of a result:
/// `frame` (its origin, x and y axes) and `normal` (x × y, the side an
/// extrusion goes to).
fn frame_json(solved: &SketchResult) -> Value {
    let normal = solved.x_axis.cross(solved.y_axis);
    json!({
        "frame": {
            "origin": [solved.origin.x, solved.origin.y, solved.origin.z],
            "x": [solved.x_axis.x, solved.x_axis.y, solved.x_axis.z],
            "y": [solved.y_axis.x, solved.y_axis.y, solved.y_axis.z],
        },
        "normal": [normal.x, normal.y, normal.z],
    })
}

fn sketch_point(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.point";
    let p = Params::new(COMMAND, params, &["sketch", "at"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let at = p.get("at").ok_or_else(|| p.missing("at"))?;
    let Some((x, y, held)) = held_coordinates(&session.typed(), COMMAND, "at", at)? else {
        return fail(format!(
            "{COMMAND}: `at` must be [x, y]: two finite numbers, or expressions such as \"-width / 2\""
        ));
    };
    let before = failing(session)?;
    let (point, held) = change(session, COMMAND, "Add point", id, |s| {
        let point = s.add_point(x, y)?;
        Ok((point, held.pin(s, point)?))
    })?;
    let held: Vec<String> = held.iter().map(ToString::to_string).collect();
    sketch_result(
        session,
        id,
        Some(&before),
        json!({ "entity": point.to_string(), "held": !held.is_empty(), "constraints": held }),
    )
}

fn sketch_line(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.line";
    let p = Params::new(COMMAND, params, &["sketch", "from", "to", "construction"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let typed = session.typed();
    let from = PointArg::required(&typed, &p, COMMAND, "from")?;
    let to = PointArg::required(&typed, &p, COMMAND, "to")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let before = failing(session)?;
    let (line, start, end) = change(session, COMMAND, "Add line", id, |s| {
        let (start, end) = (from.place(s)?, to.place(s)?);
        Ok((s.add_line(start, end, construction)?, start, end))
    })?;
    sketch_result(
        session,
        id,
        Some(&before),
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
    let typed = session.typed();
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
        corners.push(PointArg::parse(&typed, COMMAND, "points", item)?);
    }
    let before = failing(session)?;
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
        Some(&before),
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
    let typed = session.typed();
    let center = PointArg::required(&typed, &p, COMMAND, "center")?;
    let (radius, held) = match p.get("radius") {
        None => return Err(p.missing("radius")),
        Some(Value::String(text)) => {
            if text.chars().count() > MAX_COORDINATE_EXPR {
                return fail(format!(
                    "{COMMAND}: `radius` is longer than {MAX_COORDINATE_EXPR} characters"
                ));
            }
            let radius = typed
                .evaluate(text, crate::typed::TypedKind::Length)
                .map_err(|e| CommandError::new(format!("{COMMAND}: `radius`: {}", e.message())))?;
            (radius, Some(Expr::new(text.clone())))
        }
        Some(_) => (
            p.number("radius")?.ok_or_else(|| p.missing("radius"))?,
            None,
        ),
    };
    let construction = p.bool("construction")?.unwrap_or(false);
    let before = failing(session)?;
    let (circle, center, held) = change(session, COMMAND, "Add circle", id, |s| {
        let center = center.place(s)?;
        let circle = s.add_circle(center, radius, construction)?;
        let held = match held {
            Some(value) => Some(s.add_constraint(ConstraintKind::Radius {
                curve: circle,
                value,
            })?),
            None => None,
        };
        Ok((circle, center, held))
    })?;
    let held: Vec<String> = held.iter().map(ToString::to_string).collect();
    sketch_result(
        session,
        id,
        Some(&before),
        json!({ "entity": circle.to_string(), "center": center.to_string(), "constraints": held }),
    )
}

fn sketch_rect(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.rect";
    let p = Params::new(COMMAND, params, &["sketch", "from", "to", "construction"])?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let typed = session.typed();
    let from = PointArg::required(&typed, &p, COMMAND, "from")?;
    let to = PointArg::required(&typed, &p, COMMAND, "to")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let before = failing(session)?;
    let (points, lines, constraints) = change(session, COMMAND, "Add rectangle", id, |s| {
        let (Some(a), Some(c)) = (from.drawn(s), to.drawn(s)) else {
            return Err(SketchError::Invalid(
                "a corner of a rectangle cannot be an image point yet; draw it with sketch.polyline".into(),
            ));
        };
        let (width, height) = ((c.0 - a.0).abs(), (c.1 - a.1).abs());
        // Drawn with no width or height, its sides would have no direction
        // to keep horizontal or vertical.
        if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) {
            return Err(SketchError::Invalid(format!(
                "the corners are drawn {width} apart along x and {height} along y; a rectangle needs both"
            )));
        }
        let p1 = from.place(s)?;
        let p3 = to.place(s)?;
        if p1 == p3 {
            return Err(SketchError::Invalid(
                "`from` and `to` are the same point".into(),
            ));
        }
        let p2 = s.add_point(c.0, a.1)?;
        let p4 = s.add_point(a.0, c.1)?;
        let points = [p1, p2, p3, p4];
        let mut lines = Vec::with_capacity(4);
        let mut constraints = Vec::with_capacity(4);
        for (i, (start, end)) in [(p1, p2), (p2, p3), (p3, p4), (p4, p1)]
            .into_iter()
            .enumerate()
        {
            let line = s.add_line(start, end, construction)?;
            let kind = if i % 2 == 0 {
                ConstraintKind::Horizontal { a: line, b: None }
            } else {
                ConstraintKind::Vertical { a: line, b: None }
            };
            constraints.push(s.add_constraint(kind)?);
            lines.push(line);
        }
        Ok((points, lines, constraints))
    })?;
    let constraints: Vec<String> = constraints.iter().map(ToString::to_string).collect();
    sketch_result(
        session,
        id,
        Some(&before),
        json!({ "points": ids(&points), "lines": ids(&lines), "constraints": constraints }),
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
    let typed = session.typed();
    let center = PointArg::required(&typed, &p, COMMAND, "center")?;
    let start = PointArg::required(&typed, &p, COMMAND, "start")?;
    let end = PointArg::required(&typed, &p, COMMAND, "end")?;
    let construction = p.bool("construction")?.unwrap_or(false);
    let before = failing(session)?;
    let (arc, points) = change(session, COMMAND, "Add arc", id, |s| {
        let points = [center.place(s)?, start.place(s)?, end.place(s)?];
        let [c, a, b] = points;
        Ok((s.add_arc(c, a, b, construction)?, points))
    })?;
    let [center, start, end] = points;
    sketch_result(
        session,
        id,
        Some(&before),
        json!({
            "entity": arc.to_string(), "center": center.to_string(),
            "start": start.to_string(), "end": end.to_string(),
        }),
    )
}

/// A constraint's value written as a literal (no variable in it) that it
/// can never take: a length that is an angle or another quantity, a
/// negative distance, radius or diameter, an angle that is a length. Such
/// a constraint would be added and fail the sketch; it is refused at the
/// command instead. A value that uses variables is left to the
/// regeneration, as the variables may still change.
pub(crate) fn literal_problem(kind: &ConstraintKind) -> Option<String> {
    use craft3d_expr::Dimension;
    let value = kind.value()?;
    let source = value.source();
    let quantity = craft3d_expr::evaluate(source, |_| None).ok()?;
    let shown: String = source.chars().take(40).collect();
    let dimension = quantity.dimension;
    let name = kind.type_name();
    let article = if matches!(name, "angle" | "x") {
        "an"
    } else {
        "a"
    };
    let (wanted, unsigned) = match kind {
        ConstraintKind::Distance { .. }
        | ConstraintKind::Radius { .. }
        | ConstraintKind::Diameter { .. } => (Dimension::LENGTH, true),
        ConstraintKind::X { .. } | ConstraintKind::Y { .. } => (Dimension::LENGTH, false),
        ConstraintKind::Angle { .. } => (Dimension::ANGLE, false),
        _ => return None,
    };
    if dimension != wanted && !dimension.is_dimensionless() {
        return Some(format!(
            "the value `{shown}` of {article} {name} constraint is {}, but {} is needed{}",
            dimension.described(),
            if wanted == Dimension::ANGLE {
                "an angle"
            } else {
                "a length"
            },
            dimension.unit_hint(source)
        ));
    }
    if unsigned && quantity.value < 0.0 {
        return Some(format!(
            "the value `{shown}` of {article} {name} constraint is negative: a {name} cannot be negative; give its size, and draw the geometry on the side it is to lie on"
        ));
    }
    None
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
    let mut given = Vec::with_capacity(items.len());
    for item in items {
        let entity = EntityArg::parse(COMMAND, "entities", item)?;
        entity.check_instance(session, id, COMMAND)?;
        given.push(entity);
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
    // The kind is checked before anything is added; image points are made
    // in the same edit as the constraint.
    let placeholders: Vec<EntityId> = given
        .iter()
        .map(|g| match g {
            EntityArg::Id(id) => *id,
            EntityArg::Image(of, _) => *of,
        })
        .collect();
    let kind = ConstraintKind::from_parts(type_name, &placeholders, value.clone(), direction)
        .map_err(|reason| {
            let shown: String = reason.chars().take(400).collect();
            CommandError::new(format!("{COMMAND}: {shown}"))
        })?;
    if let Some(problem) = literal_problem(&kind) {
        return fail(format!("{COMMAND}: {problem}"));
    }
    let label = format!("Constrain {}", kind.type_name());
    let before = failing(session)?;
    let constraint = change(session, COMMAND, &label, id, |s| {
        let mut entities = Vec::with_capacity(given.len());
        for entity in &given {
            entities.push(entity.place(s)?);
        }
        let kind = ConstraintKind::from_parts(type_name, &entities, value, direction)
            .map_err(SketchError::Invalid)?;
        s.add_constraint(kind)
    })?;
    sketch_result(
        session,
        id,
        Some(&before),
        json!({ "constraint": constraint.to_string() }),
    )
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
    let before = failing(session)?;
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
            if let Some(problem) = literal_problem(kind) {
                return Err(SketchError::Invalid(format!("{constraint}: {problem}")));
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
            // A face given again keeps the x axis it was turned to.
            let x_axis = match &s.plane {
                SketchPlane::Face { x_axis, .. } => x_axis.clone(),
                _ => None,
            };
            s.plane = SketchPlane::Face {
                face,
                offset: kept,
                x_axis,
            };
        }
        if let Some(offset) = offset {
            *s.plane.offset_mut() = offset;
        }
        Ok(())
    })?;
    sketch_result(session, id, Some(&before), Value::Null)
}

fn sketch_delete(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.delete";
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "entity", "constraint", "pattern"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    if let Some(value) = p.get("pattern") {
        if p.get("entity").is_some() || p.get("constraint").is_some() {
            return fail(format!(
                "{COMMAND}: give one of `entity`, `constraint` or `pattern` to delete"
            ));
        }
        let pattern = crate::sketch_pattern_cmds::pattern_id(COMMAND, "pattern", value)?;
        let before = failing(session)?;
        let removal = change(session, COMMAND, "Delete pattern", id, |s| {
            s.remove_pattern(pattern)
        })?;
        let own = removal_json(&removal, json!({ "deleted": pattern.to_string() }));
        return sketch_result(session, id, Some(&before), own);
    }
    let entity = p
        .get("entity")
        .map(|v| entity_id(COMMAND, "entity", v))
        .transpose()?;
    let constraint = p
        .get("constraint")
        .map(|v| constraint_id(COMMAND, "constraint", v))
        .transpose()?;
    let before = failing(session)?;
    let own = match (entity, constraint) {
        (Some(entity), None) => {
            let removal = change(session, COMMAND, "Delete entity", id, |s| {
                s.remove_entity_reporting(entity)
            })?;
            removal_json(&removal, json!({ "deleted": entity.to_string() }))
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
    sketch_result(session, id, Some(&before), own)
}

/// What a deletion took with it, added to `own`: the constraints always,
/// image points and patterns when there were any.
fn removal_json(removal: &Removal, mut own: Value) -> Value {
    let names = |list: Vec<String>| Value::from(list);
    if let Some(map) = own.as_object_mut() {
        map.insert(
            "constraintsDeleted".into(),
            names(
                removal
                    .constraints
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
        );
        if !removal.images.is_empty() {
            map.insert("imagesDeleted".into(), names(ids(&removal.images)));
        }
        if !removal.patterns.is_empty() {
            map.insert(
                "patternsDeleted".into(),
                names(removal.patterns.iter().map(ToString::to_string).collect()),
            );
        }
        if !removal.unwrapped.is_empty() {
            let taken: Vec<Value> = removal
                .unwrapped
                .iter()
                .map(|(inner, outer)| json!({"pattern": inner.to_string(), "from": outer.to_string()}))
                .collect();
            map.insert("takenOutOfSeeds".into(), taken.into());
        }
    }
    own
}

/// How regeneration fitted a formula curve or a live spline: the size of
/// the spline and, for a formula, how far it was found from it.
fn fit_json(fitted: &craft3d_regen::FittedCurve) -> Value {
    let mut fit = json!({
        "degree": fitted.spline.degree,
        "controlPoints": fitted.spline.control.len(),
    });
    if let (Some(map), Some(deviation)) = (fit.as_object_mut(), fitted.deviation) {
        map.insert("deviation".into(), deviation.into());
    }
    fit
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
                    Geometry::Formula {
                        ref x,
                        ref y,
                        ref from,
                        ref to,
                        start,
                        end,
                    } => {
                        map.insert("x".into(), x.source().into());
                        map.insert("y".into(), y.source().into());
                        map.insert("from".into(), from.source().into());
                        map.insert("to".into(), to.source().into());
                        map.insert("start".into(), start.to_string().into());
                        map.insert("end".into(), end.to_string().into());
                        map.insert("closed".into(), (start == end).into());
                        if let Some(fitted) = solved.and_then(|s| s.curve(entity.id)) {
                            map.insert("fit".into(), fit_json(fitted));
                        }
                    }
                    Geometry::Interpolated {
                        degree,
                        ref through,
                    } => {
                        map.insert("degree".into(), degree.into());
                        map.insert("through".into(), ids(through).into());
                        map.insert(
                            "closed".into(),
                            (through.len() > 1 && through.first() == through.last()).into(),
                        );
                        if let Some(fitted) = solved.and_then(|s| s.curve(entity.id)) {
                            map.insert("fit".into(), fit_json(fitted));
                        }
                    }
                    Geometry::Text(ref text) => {
                        map.insert(
                            "text".into(),
                            crate::text_cmds::text_json(entity.id, text, solved.map(|s| &**s)),
                        );
                    }
                    Geometry::Image { of, pattern, index } => {
                        map.insert("of".into(), of.to_string().into());
                        map.insert("pattern".into(), pattern.to_string().into());
                        map.insert("index".into(), index.to_string().into());
                        map.insert("ref".into(), format!("{of}[{index}]").into());
                        let result =
                            solved.and_then(|s| s.patterns.iter().find(|r| r.id == pattern));
                        match result.map(|r| (r, r.resolve(index))) {
                            Some((_, Some((i, j)))) => {
                                let at = if result.is_some_and(|r| r.grid) {
                                    json!([i, j])
                                } else {
                                    json!(i)
                                };
                                map.insert("instance".into(), at);
                            }
                            Some((r, None)) => {
                                map.insert("missing".into(), r.describe().into());
                            }
                            None => {}
                        }
                        if let Some((x, y)) = solved.and_then(|s| s.point(entity.id)) {
                            map.insert("x".into(), x.into());
                            map.insert("y".into(), y.into());
                        }
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
            if let Value::Object(place) = frame_json(solved) {
                map.extend(place);
            }
        }
        map.insert(
            "plane".into(),
            serde_json::to_value(&sketch.plane).unwrap_or(Value::Null),
        );
        map.insert("entities".into(), entities.into());
        map.insert("constraints".into(), constraints.into());
        if !sketch.patterns.is_empty() {
            map.insert(
                "patterns".into(),
                crate::sketch_pattern_cmds::patterns_json(sketch, solved.map(AsRef::as_ref)).into(),
            );
        }
        map.insert("lengthUnit".into(), doc.units.length.symbol().into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::number_expr;

    /// A number held beside an expression is written short and reads back
    /// to exactly itself.
    #[test]
    fn a_held_number_is_written_short_and_reads_back_exactly() {
        let values = [
            0.0,
            -0.0,
            7.0,
            12.5,
            -0.1,
            1.0 / 3.0,
            1e300,
            -1e300,
            2.5e-7,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            123_456_789.125,
        ];
        for v in values {
            let text = number_expr(v);
            assert!(text.len() <= 24, "{v}: {text}");
            let back = craft3d_expr::evaluate(&text, |_| None).unwrap().value;
            assert_eq!(back.to_bits(), v.to_bits(), "{v}: {text}");
        }
        assert_eq!(number_expr(1e300), "1e300");
        assert_eq!(number_expr(12.5), "12.5");
        assert_eq!(number_expr(0.0), "0");
    }
}
