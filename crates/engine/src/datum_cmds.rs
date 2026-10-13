//! Reference geometry: datum planes as features of the history.
//!
//! A datum makes no body. It is placed relative to a base plane, another
//! datum, or a face, an edge or a vertex of a body, and what it is placed
//! by is found again at every regeneration: a face and a vertex by their
//! persistent names, an edge by the two faces it lies between. Whether
//! the references resolve is found out by regeneration, like every other
//! parameter: the datum is stored and reports `failed` with the reason.

use crate::commands::{Run, edit, exprs, fail, feature_ref, feature_result, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    AxisRef, DatumPlane, Document, Expr, FeatureId, FeatureKind, PersistentName, PlaneBy, PlaneRef,
    PointRef, WorldAxis, WorldPlane,
};
use craft3d_math::Vec3;
use craft3d_regen::Datum;
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("datum.plane", "Datum Plane", r#"{"from": "xy|yz|zx" or face name or datum plane, "distance": expr, "angle": expr, "about": "x|y|z" or [face name, face name], "through": [point, point, point], "replace": id or name, "name": string = automatic}"#,
        "Adds a datum plane: a plane to sketch on (sketch.create, plane) and to mirror in (feature.mirror, plane). It makes no body. Three ways. At a distance: from and distance; from is a base plane, a flat face by its name from document.inspect (faceNames; its normal points out of the body) or an earlier datum plane (id or name), and the plane keeps the origin and axes of from, moved along its normal. At an angle: from, about and angle; about is a world axis or the straight edge between two faces of one body ([face name, face name]; between two flat faces it points along the cross product of their outward normals in that order), it has to be parallel to from, and the plane is from turned by angle about it (counter-clockwise seen from the tip of the axis) and goes through it. Through three points: through, each point [x, y, z] (expressions) or a vertex name such as \"F1:vertex(6)\" (the vertices of a block are numbered x + 2 y + 4 z, each 0 on the low and 1 on the high side in its own axes: 6 is the corner at low x, high y, high z); the origin is the first point, x points at the second and the three go counter-clockwise round the normal. distance and angle are expressions and follow variables. A reference that is lost after an upstream edit fails the datum with the reason and the name it lost, and what is placed by the datum fails naming it; everything else is still built. replace gives an existing datum plane this definition instead of adding one. The result has the plane under datum: origin, normal, x.", doc: true, datum_plane),
];

/// A feature that has to be reference geometry of one kind (`wanted` is
/// the kind's type name, such as `datumPlane`), by id or name.
pub(crate) fn datum_ref(
    doc: &Document,
    p: &Params,
    command: &str,
    key: &str,
    wanted: &str,
) -> Result<FeatureId, CommandError> {
    let id = feature_ref(doc, p, command, key)?;
    datum_kind(doc, id, command, key, wanted)?;
    Ok(id)
}

fn datum_kind(
    doc: &Document,
    id: FeatureId,
    command: &str,
    key: &str,
    wanted: &str,
) -> Result<(), CommandError> {
    match doc.feature(id) {
        Some(feature) if feature.kind.type_name() == wanted => Ok(()),
        Some(feature) => fail(format!(
            "{command}: `{key}`: `{}` is a {}, not a {wanted}",
            feature.name,
            feature.kind.type_name()
        )),
        None => fail(format!("{command}: there is no feature {id}")),
    }
}

/// A feature by the text of a parameter: its name, or its id as `F3`.
fn feature_named(doc: &Document, text: &str) -> Option<FeatureId> {
    doc.feature_by_name(text).map(|f| f.id).or_else(|| {
        let id = FeatureId(text.strip_prefix('F')?.parse().ok()?);
        doc.feature(id).map(|f| f.id)
    })
}

fn shown(text: &str) -> String {
    text.chars().take(60).collect()
}

/// A persistent name, as `document.inspect` lists it.
fn name_value(
    command: &str,
    key: &str,
    value: &Value,
    what: &str,
) -> Result<PersistentName, CommandError> {
    let Value::String(text) = value else {
        return fail(format!("{command}: `{key}` must be {what}"));
    };
    text.parse::<PersistentName>()
        .map_err(|reason| CommandError::new(format!("{command}: `{key}`: {reason}")))
}

/// A face by its name.
fn face_value(command: &str, key: &str, value: &Value) -> Result<PersistentName, CommandError> {
    let name = name_value(command, key, value, "a face name such as \"F2:endCap(e1)\"")?;
    if !name.is_face() {
        return fail(format!(
            "{command}: `{key}`: {name} is not a face (see faceNames in document.inspect)"
        ));
    }
    Ok(name)
}

/// Two faces of one body: the edge between them.
fn between_value(
    command: &str,
    key: &str,
    value: &Value,
) -> Result<[PersistentName; 2], CommandError> {
    let pair = value.as_array().map(Vec::as_slice);
    let Some([a, b]) = pair else {
        return fail(format!(
            "{command}: `{key}` must be two face names [face, face]: the edge between them"
        ));
    };
    Ok([face_value(command, key, a)?, face_value(command, key, b)?])
}

/// A plane: a base plane, a face by its name, or a datum plane by id or
/// name. A base plane's name wins over a feature of the same name.
pub(crate) fn plane_value(
    doc: &Document,
    command: &str,
    key: &str,
    value: &Value,
) -> Result<PlaneRef, CommandError> {
    let by_id = |id: FeatureId| -> Result<PlaneRef, CommandError> {
        datum_kind(doc, id, command, key, "datumPlane")?;
        Ok(PlaneRef::Datum { feature: id })
    };
    match value {
        Value::Number(n) => match n.as_u64().map(FeatureId) {
            Some(id) => by_id(id),
            None => fail(format!("{command}: `{key}` must be a plane, got {n}")),
        },
        Value::String(text) => {
            if let Some(plane) = WorldPlane::from_name(text) {
                return Ok(PlaneRef::World { plane });
            }
            if let Some(id) = feature_named(doc, text) {
                return by_id(id);
            }
            if text.contains(':') {
                return Ok(PlaneRef::Face {
                    face: face_value(command, key, value)?,
                });
            }
            fail(format!(
                "{command}: `{key}`: there is no plane `{}`; give xy, yz or zx, a face name (faceNames in document.inspect) or a datum plane",
                shown(text)
            ))
        }
        _ => fail(format!(
            "{command}: `{key}` must be a plane: xy, yz or zx, a face name or a datum plane"
        )),
    }
}

/// An axis: a world axis, or the straight edge between two faces.
fn axis_value(command: &str, key: &str, value: &Value) -> Result<AxisRef, CommandError> {
    match value {
        Value::String(text) => match WorldAxis::from_name(text) {
            Some(axis) => Ok(AxisRef::World { axis }),
            None => fail(format!(
                "{command}: `{key}`: there is no axis `{}`; give x, y or z, or two face names [face, face] for the edge between them",
                shown(text)
            )),
        },
        Value::Array(_) => Ok(AxisRef::Edge {
            between: between_value(command, key, value)?,
        }),
        _ => fail(format!(
            "{command}: `{key}` must be an axis: x, y or z, or two face names [face, face] for the edge between them"
        )),
    }
}

/// A point: coordinates `[x, y, z]`, or a vertex by its name.
fn point_value(p: &Params, key: &str, value: &Value) -> Result<PointRef, CommandError> {
    let command = p.command();
    match value {
        Value::Array(items) => {
            let [x, y, z] = items.as_slice() else {
                return fail(format!(
                    "{command}: `{key}`: a point has three coordinates [x, y, z]"
                ));
            };
            Ok(PointRef::At {
                position: exprs([p.expr_of(key, x)?, p.expr_of(key, y)?, p.expr_of(key, z)?]),
            })
        }
        Value::String(_) => Ok(PointRef::Vertex {
            vertex: name_value(
                command,
                key,
                value,
                "a vertex name such as \"F1:vertex(6)\"",
            )?,
        }),
        _ => fail(format!(
            "{command}: `{key}`: a point is [x, y, z] or a vertex name such as \"F1:vertex(6)\""
        )),
    }
}

fn vec3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

/// Where a datum lies, as the commands report it.
pub(crate) fn datum_json(datum: &Datum) -> Value {
    match datum {
        Datum::Plane(frame) => json!({
            "kind": "plane",
            "origin": vec3(frame.origin()),
            "normal": vec3(frame.z()),
            "x": vec3(frame.x()),
        }),
    }
}

/// What a datum command returns: the feature, how it regenerated, and
/// where the datum lies.
fn datum_result(session: &mut Session, id: FeatureId) -> Run {
    let mut result = feature_result(session, id)?;
    let regen = session.document_mut()?.regen();
    if let (Some(map), Some(datum)) = (
        result.as_object_mut(),
        regen.feature(id).and_then(|f| f.datum.as_ref()),
    ) {
        map.insert("datum".into(), datum_json(datum));
    }
    Ok(result)
}

/// Adds the datum, or gives the datum `replace` names this definition.
fn store(session: &mut Session, command: &'static str, p: &Params, kind: FeatureKind) -> Run {
    let name = p.str("name")?;
    let id = if p.get("replace").is_some() {
        if name.is_some() {
            return fail(format!(
                "{command}: `name` names a new datum; rename an existing one with feature.rename"
            ));
        }
        let doc = session.document()?.history.current();
        let id = datum_ref(doc, p, command, "replace", kind.type_name())?;
        edit(session, command, "Edit datum", |doc| {
            doc.feature_mut(id)?.kind = kind;
            Ok(id)
        })?
    } else {
        edit(session, command, "Add datum", |doc| {
            doc.add_feature(name, kind)
        })?
    };
    datum_result(session, id)
}

fn datum_plane(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "datum.plane";
    let p = Params::new(
        ID,
        params,
        &[
            "from", "distance", "angle", "about", "through", "replace", "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let distance = p.expr("distance")?.map(Expr::new);
    let angle = p.expr("angle")?.map(Expr::new);
    let about = match p.get("about") {
        None => None,
        Some(value) => Some(axis_value(ID, "about", value)?),
    };
    let from = match p.get("from") {
        None => None,
        Some(value) => Some(plane_value(doc, ID, "from", value)?),
    };
    let through = match p.get("through") {
        None => None,
        Some(value) => {
            let Some([a, b, c]) = value.as_array().map(Vec::as_slice) else {
                return fail(format!(
                    "{ID}: `through` must be three points [point, point, point]"
                ));
            };
            Some([
                point_value(&p, "through", a)?,
                point_value(&p, "through", b)?,
                point_value(&p, "through", c)?,
            ])
        }
    };
    let by = match (from, through) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{ID}: give either `from` (with `distance`, or with `about` and `angle`) or `through`, not both"
            ));
        }
        (None, None) => {
            return fail(format!(
                "{ID}: missing parameter `from` (a plane, with `distance`, or with `about` and `angle`) or `through` (three points)"
            ));
        }
        (None, Some(points)) => {
            if let Some(key) = ["distance", "angle", "about"]
                .into_iter()
                .find(|k| p.has(k))
            {
                return fail(format!(
                    "{ID}: `{key}` goes with `from`, not with `through`"
                ));
            }
            PlaneBy::ThreePoints { points }
        }
        (Some(from), None) => match (distance, about, angle) {
            (Some(distance), None, None) => PlaneBy::Offset { from, distance },
            (None, Some(about), Some(angle)) => PlaneBy::Angle { from, about, angle },
            (Some(_), _, _) => {
                return fail(format!(
                    "{ID}: give either `distance` (a plane beside `from`) or `about` and `angle` (a plane turned from it), not both"
                ));
            }
            (None, Some(_), None) => return Err(p.missing("angle")),
            (None, None, Some(_)) => return Err(p.missing("about")),
            (None, None, None) => {
                return fail(format!(
                    "{ID}: missing parameter `distance` (or `about` and `angle`): where the plane lies relative to `from`"
                ));
            }
        },
    };
    store(session, ID, &p, FeatureKind::DatumPlane(DatumPlane { by }))
}
