//! A body cut by planes in one feature, one side of each kept
//! (`feature.planeCut`; docs/design/plane-cut.md).
//!
//! The target is a feature (id or name); its body goes into the cut,
//! unless `keep` is given. A plane is a base plane, a datum plane, a flat
//! face by its name, or a point and a normal given as expressions; each
//! has an id of its own in the feature, which names the face it cuts, so
//! that the name stays with its plane when planes are added, deleted or
//! put in another order. Whether the planes can be applied is found out
//! by regeneration, like every other parameter: the feature is stored and
//! reports `failed` with the plane and the reason.

use crate::commands::{Run, add_feature, exprs, fail, feature_ref, origin, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    CutPlane, CutPlaneBy, CutSide, Document, Expr, FeatureKind, MAX_CUT_PLANE_ID, MAX_CUT_PLANES,
    PlaneCut,
};
use craft3d_regen::CutReport;
use serde_json::{Map, Value, json};

/// The command's id.
pub(crate) const ID: &str = "feature.planeCut";

/// What a plane given as an object may hold.
const PLANE_KEYS: [&str; 6] = ["plane", "point", "normal", "offset", "side", "id"];

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "feature.planeCut",
    "Cut by Planes",
    r#"{"target": id or name, "planes": [plane, ...] where plane is "xy|yz|zx" or datum plane (id or name) or a flat face by name, or {plane: one of those, offset: expr = 0, side: below|above = below, id: number = a new one}, or {point: [expr, expr, expr] = [0, 0, 0], normal: [expr, expr, expr], offset: expr = 0, side: below|above = below, id: number = a new one}, "keep": boolean = false, "name": string = automatic, "when": expr = always on}"#,
    "Adds a feature whose body is the body of target cut by planes, one side of each kept: what lies on the kept side of every plane (several planes: the intersection of what each keeps). No tool is drawn or sized: the cut reaches as far as the body does. A plane is a base plane, a datum plane (the cut follows it), a flat face of an earlier body by its name (its normal points out of that body), or a point and a normal given as expressions (the point three lengths, the normal three plain numbers or three lengths of any length, so both can follow variables; a plane a x + b y + c z = d is normal [a, b, c] through the point [a d / s, b d / s, c d / s] with s = a a + b b + c c), each moved by offset along its normal. side says what is kept: below (the default) is the side the normal points away from, as measure.below has it, above the side it points to. One plane may be given alone instead of a list; at most 256. The target's body goes into this feature; with keep it keeps its body and this is a cut copy (a kept copy cut once on each side gives both parts). Each plane has an id (given, or the next free one from 1; never one a deleted plane had), and the face it cuts is named by it, \"F9:cut(3)\", so the name stays with its plane when planes are added, deleted or put in another order; a plane that cuts the body in several places gives several faces of that name, and a piece that joins a face of the target lying in the plane is that face, answering to both names. What is left of the target's faces keeps its names with a prefix, \"F9:copy(0)/...\"; a name of the target's body is followed into the cut body by the features after it, as through every feature that takes a body. The result lists the planes under planes ({id, cuts, faces}: whether the cut body has a face of that plane, and how many); one that has none (it does not meet the body, lies in a face of it with the body behind, or what it cut was cut away by the others) changes nothing and is listed under warnings. A plane that leaves nothing of the body fails the feature, naming the plane. The planes are applied in the order given, each a cut of its own: nothing is snapped, so planes that nearly meet in a point leave the small faces between them, and where faces or points come between half and twice the document's tolerance of each other the feature fails with the plane and the faces (planes meant to meet in one point: give them the same point), also for a place a later plane of the list would cut away (put that plane first). Refused as well: a sheet as the target, a face that is not flat, a normal of no length, parts left touching along an edge or in a point. feature.edit changes target, planes (the whole list: a plane given with its id stays that plane; one given without is the plane it equals in every field, else a new plane with a new id) and keep.",
    doc: true,
    names: ["planes"],
    feature_plane_cut
)];

/// Three expressions inside a plane: `[x, y, z]`.
fn triple(p: &Params, key: &str, value: &Value) -> Result<[String; 3], CommandError> {
    let command = p.command();
    let Some([x, y, z]) = value.as_array().map(Vec::as_slice) else {
        return fail(format!(
            "{command}: `{key}` must be an array of three expressions [x, y, z]"
        ));
    };
    Ok([p.expr_of(key, x)?, p.expr_of(key, y)?, p.expr_of(key, z)?])
}

/// One plane as a command gives it; its id 0 when none was given.
fn plane_value(
    doc: &Document,
    p: &Params,
    index: usize,
    value: &Value,
) -> Result<CutPlane, CommandError> {
    let command = p.command();
    let key = format!("planes.{index}");
    let referred = |key: &str, value: &Value| -> Result<CutPlaneBy, CommandError> {
        Ok(CutPlaneBy::Plane {
            plane: crate::datum_cmds::plane_value(doc, command, key, value)?,
        })
    };
    let map = match value {
        Value::String(_) | Value::Number(_) => {
            return Ok(CutPlane {
                id: 0,
                by: referred(&key, value)?,
                offset: Expr::new("0"),
                side: CutSide::Below,
            });
        }
        Value::Object(map) => map,
        _ => {
            return fail(format!(
                "{command}: `{key}` must be a plane: xy, yz or zx, a datum plane, a flat face by its name, or an object with `plane`, or with `normal` and `point`"
            ));
        }
    };
    if let Some(unknown) = map.keys().find(|k| !PLANE_KEYS.contains(&k.as_str())) {
        return fail(format!(
            "{command}: `{key}`: unknown key `{unknown}`; a plane has {}",
            PLANE_KEYS.join(", ")
        ));
    }
    let given = |name: &str| map.get(name).filter(|v| !v.is_null());
    let by = match (given("plane"), given("normal")) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{command}: `{key}`: give either `plane` (a base plane, a datum plane or a flat face) or `normal` (with `point`), not both"
            ));
        }
        (Some(plane), None) => {
            if given("point").is_some() {
                return fail(format!(
                    "{command}: `{key}`: `point` goes with `normal`, not with `plane`: the plane says where it lies (move it with `offset`)"
                ));
            }
            referred(&format!("{key}.plane"), plane)?
        }
        (None, Some(normal)) => CutPlaneBy::PointNormal {
            point: match given("point") {
                Some(point) => exprs(triple(p, &format!("{key}.point"), point)?),
                None => exprs(origin()),
            },
            normal: exprs(triple(p, &format!("{key}.normal"), normal)?),
        },
        (None, None) => {
            return fail(format!(
                "{command}: `{key}` needs `plane` (a base plane, a datum plane or a flat face) or `normal` (with `point`)"
            ));
        }
    };
    let offset = match given("offset") {
        Some(offset) => p.expr_of(&format!("{key}.offset"), offset)?,
        None => "0".to_string(),
    };
    let side = match given("side") {
        None => CutSide::Below,
        Some(side) => side
            .as_str()
            .and_then(CutSide::from_name)
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{command}: `{key}.side` must be \"below\" (keep the side the normal points away from) or \"above\""
                ))
            })?,
    };
    let id = match given("id") {
        None => 0,
        Some(id) => id
            .as_u64()
            .filter(|&n| n > 0 && n <= u64::from(MAX_CUT_PLANE_ID))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| {
                CommandError::new(format!(
                    "{command}: `{key}.id` is a plane's id (a whole number from 1 to {MAX_CUT_PLANE_ID}, as the feature's result lists them under planes)"
                ))
            })?,
    };
    Ok(CutPlane {
        id,
        by,
        offset: Expr::new(offset),
        side,
    })
}

/// The planes of `planes`: a list, or one plane alone. Ids not given are
/// 0.
fn planes_value(doc: &Document, p: &Params) -> Result<Vec<CutPlane>, CommandError> {
    let command = p.command();
    let value = p.get("planes").ok_or_else(|| p.missing("planes"))?;
    let alone = [value];
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        _ => alone.to_vec(),
    };
    if items.is_empty() {
        return fail(format!(
            "{command}: `planes` is empty; give at least one plane"
        ));
    }
    if items.len() > MAX_CUT_PLANES {
        return fail(format!(
            "{command}: `planes` has {} planes; the most is {MAX_CUT_PLANES}",
            items.len()
        ));
    }
    let mut planes = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        planes.push(plane_value(doc, p, index, item)?);
    }
    Ok(planes)
}

/// Refuses an id two planes of the list were given.
fn no_id_twice(command: &str, planes: &[CutPlane]) -> Result<(), CommandError> {
    let mut seen = std::collections::BTreeSet::new();
    for (index, plane) in planes.iter().enumerate() {
        if plane.id != 0 && !seen.insert(plane.id) {
            return fail(format!(
                "{command}: `planes.{index}.id`: the id {} is given to two planes; every plane has an id of its own",
                plane.id
            ));
        }
    }
    Ok(())
}

fn feature_plane_cut(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(ID, params, &["target", "planes", "keep", "name"])?;
    let doc = session.document()?.history.current();
    let target = feature_ref(doc, &p, ID, "target")?;
    let planes = planes_value(doc, &p)?;
    no_id_twice(ID, &planes)?;
    let keep = p.bool("keep")?.unwrap_or(false);
    let Some(cut) = PlaneCut::new(target, planes, keep) else {
        return fail(format!(
            "{ID}: the planes' ids run past the highest, {MAX_CUT_PLANE_ID}"
        ));
    };
    add_feature(session, ID, p.str("name")?, FeatureKind::PlaneCut(cut))
}

/// `feature.edit` for a plane cut: `planes`, the whole list. A plane
/// given with its id is the cut's plane of that id; one given without is
/// the plane it equals in every field (the first not taken yet), and else
/// a new plane with an id no plane of the cut had. (`target` and `keep`
/// are applied by `feature.edit` itself.)
pub(crate) fn edit(doc: &Document, p: &Params, cut: &mut PlaneCut) -> Result<(), CommandError> {
    if p.get("planes").is_none() {
        return Ok(());
    }
    let command = p.command();
    let mut planes = planes_value(doc, p)?;
    no_id_twice(command, &planes)?;
    let mut taken: Vec<u32> = Vec::with_capacity(planes.len());
    for (index, plane) in planes.iter().enumerate() {
        if plane.id == 0 {
            continue;
        }
        if !cut.planes.iter().any(|old| old.id == plane.id) {
            let has: Vec<String> = cut.planes.iter().map(|old| old.id.to_string()).collect();
            return fail(format!(
                "{command}: `planes.{index}.id`: the cut has no plane {}; its planes are {} (leave the id out for a new plane)",
                plane.id,
                has.join(", ")
            ));
        }
        taken.push(plane.id);
    }
    for plane in &mut planes {
        if plane.id != 0 {
            continue;
        }
        let same = cut
            .planes
            .iter()
            .find(|old| !taken.contains(&old.id) && old.same_but_id(plane));
        if let Some(old) = same {
            plane.id = old.id;
            taken.push(old.id);
        }
    }
    cut.planes = planes;
    if cut.number_new_planes().is_none() {
        return fail(format!(
            "{command}: the planes' ids run past the highest, {MAX_CUT_PLANE_ID}"
        ));
    }
    Ok(())
}

/// What each plane of a cut left, into a feature's result: `planes`, per
/// plane its id, whether the cut body has a face of it and how many.
pub(crate) fn report_json(map: &mut Map<String, Value>, reports: &[CutReport]) {
    let planes: Vec<Value> = reports
        .iter()
        .map(|r| json!({ "id": r.id, "cuts": r.faces > 0, "faces": r.faces }))
        .collect();
    map.insert("planes".into(), planes.into());
}

/// [`report_json`], and under `warnings` every plane the cut body has no
/// face of.
pub(crate) fn report_with_warnings(map: &mut Map<String, Value>, reports: &[CutReport]) {
    report_json(map, reports);
    let idle: Vec<Value> = reports
        .iter()
        .filter(|r| r.faces == 0)
        .map(|r| {
            format!(
                "plane {} cuts nothing: the body has no face of it (it does not meet the body, lies in a face of it with the body behind, or what it cut was cut away by the other planes)",
                r.id
            )
            .into()
        })
        .collect();
    if idle.is_empty() {
        return;
    }
    if let Some(warnings) = map
        .entry("warnings")
        .or_insert_with(|| json!([]))
        .as_array_mut()
    {
        warnings.extend(idle);
    }
}
