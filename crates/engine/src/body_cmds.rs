//! Features that make a body out of the body of an earlier feature: move
//! it, mirror it, repeat it.
//!
//! The source is a feature (id or name). Its body goes into the new
//! feature and is no longer listed on its own, unless `keep` is given.
//! Whether the source has a body at all is found out by regeneration, like
//! every other parameter: the feature is stored and reports `failed` with
//! the reason.

use crate::commands::{Run, add_feature, exprs, fail, feature_ref, origin, required, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::{
    Boolean, BooleanOp, Expr, FeatureKind, Mirror, Pattern, PatternLayout, Transform,
};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.transform", "Move Body", r#"{"source": id or name, "translate": [expr, expr, expr] = [0, 0, 0], "angle": expr = "0 deg", "axis": [number, number, number] = [0, 0, 1], "pivot": [expr, expr, expr] = [0, 0, 0], "keep": boolean = false, "name": string = automatic}"#,
        "Adds a feature whose body is the body of source, turned by angle about the line through pivot along axis (counter-clockwise seen from the tip of axis) and then shifted by translate. The source's body goes into it; with keep the source keeps its body and this is a moved copy. Faces keep their names with a prefix: \"F5:copy(0)/F2:side(e7)\".", doc: true, feature_transform),
    spec!("feature.mirror", "Mirror Body", r#"{"source": id or name, "normal": [number, number, number], "point": [expr, expr, expr] = [0, 0, 0], "plane": datum plane id or name, "keep": boolean = false, "name": string = automatic}"#,
        "Adds a feature whose body is the body of source mirrored in a plane: the plane through point with the given normal, or a datum plane (plane, instead of normal and point; the mirror follows the datum). With keep the source keeps its body and this is a mirrored copy.", doc: true, feature_mirror),
    spec!("feature.pattern", "Pattern", r#"{"source": id or name, "count": expr, "step": [expr, expr, expr], "angle": expr, "axis": [number, number, number] or datum axis = [0, 0, 1], "point": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a feature whose body is count instances of the body of source, the source included. Linear: give step, the shift from one instance to the next. Circular: give angle, the turn from one instance to the next about the line through point along axis (\"360 deg / n\" with count n goes evenly round), or about a datum axis (axis: its name or id, without point; the pattern follows the datum). count is a whole number and may be a formula of variables. Instances that touch or overlap are joined into one piece. The faces of instance i are named \"F5:copy(i)/...\".", doc: true, feature_pattern),
    spec!("feature.boolean", "Boolean", r#"{"op": "union|subtract|intersect", "target": id or name, "tool": id or name, "keepTool": boolean = false, "name": string = automatic}"#,
        "Adds a feature whose body is the bodies of target and tool combined: union is everything in either, subtract is the target without what the tool covers, intersect is what is in both. Both bodies go into it; with keepTool the tool keeps its body, to use it again. Faces keep their names with a prefix: \"F7:copy(0)/...\" for a face of the target, \"F7:copy(1)/...\" for one of the tool. What the kernel cannot combine yet fails with the reason and the faces involved.", doc: true, feature_boolean),
];

fn triple(
    p: &Params,
    key: &str,
    default: fn() -> [String; 3],
) -> Result<[Expr; 3], crate::CommandError> {
    Ok(exprs(p.expr3(key)?.unwrap_or_else(default)))
}

fn z_axis() -> [String; 3] {
    ["0".to_string(), "0".to_string(), "1".to_string()]
}

fn feature_transform(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.transform";
    let p = Params::new(
        ID,
        params,
        &[
            "source",
            "translate",
            "angle",
            "axis",
            "pivot",
            "keep",
            "name",
        ],
    )?;
    let source = feature_ref(session.document()?.history.current(), &p, ID, "source")?;
    let kind = FeatureKind::Transform(Transform {
        source,
        translate: triple(&p, "translate", origin)?,
        pivot: triple(&p, "pivot", origin)?,
        axis: triple(&p, "axis", z_axis)?,
        angle: Expr::new(p.expr("angle")?.unwrap_or_else(|| "0 deg".to_string())),
        keep: p.bool("keep")?.unwrap_or(false),
    });
    add_feature(session, ID, p.str("name")?, kind)
}

fn feature_mirror(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.mirror";
    let p = Params::new(
        ID,
        params,
        &["source", "normal", "point", "plane", "keep", "name"],
    )?;
    let doc = session.document()?.history.current();
    let source = feature_ref(doc, &p, ID, "source")?;
    let plane = match p.get("plane") {
        None => None,
        Some(_) => Some(crate::datum_cmds::datum_ref(
            doc,
            &p,
            ID,
            "plane",
            "datumPlane",
        )?),
    };
    let normal = match (p.expr3("normal")?, plane) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{ID}: give either `plane` (a datum plane) or `normal` (with `point`), not both"
            ));
        }
        (Some(normal), None) => normal,
        // Kept, unused: the plane a mirror without a datum goes back to.
        (None, Some(_)) => z_axis(),
        (None, None) => {
            return fail(format!(
                "{ID}: missing parameter `normal` (or `plane`: a datum plane to mirror in)"
            ));
        }
    };
    if plane.is_some() && p.has("point") {
        return fail(format!(
            "{ID}: `point` goes with `normal`, not with `plane`: a datum plane says where it lies"
        ));
    }
    let kind = FeatureKind::Mirror(Mirror {
        source,
        point: triple(&p, "point", origin)?,
        normal: exprs(normal),
        plane,
        keep: p.bool("keep")?.unwrap_or(false),
    });
    add_feature(session, ID, p.str("name")?, kind)
}

fn feature_pattern(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.pattern";
    let p = Params::new(
        ID,
        params,
        &["source", "count", "step", "angle", "axis", "point", "name"],
    )?;
    let doc = session.document()?.history.current();
    let source = feature_ref(doc, &p, ID, "source")?;
    let count = required(&p, "count")?;
    let layout = match (p.expr3("step")?, p.expr("angle")?) {
        (Some(_), Some(_)) => {
            return fail(format!(
                "{ID}: give either `step` (a linear pattern) or `angle` (a circular one), not both"
            ));
        }
        (None, None) => {
            return fail(format!(
                "{ID}: give `step` (the shift between instances of a linear pattern) or `angle` (the turn between instances of a circular one)"
            ));
        }
        (Some(step), None) => {
            if let Some(key) = ["axis", "point"].into_iter().find(|key| p.has(key)) {
                return fail(format!(
                    "{ID}: `{key}` belongs to a circular pattern (with `angle`), not to one with `step`"
                ));
            }
            PatternLayout::Linear { step: exprs(step) }
        }
        (None, Some(angle)) => {
            // A datum axis by name, or a direction with a point.
            let about = match p.get("axis") {
                Some(Value::String(_)) => Some(crate::datum_cmds::datum_ref(
                    doc,
                    &p,
                    ID,
                    "axis",
                    "datumAxis",
                )?),
                _ => None,
            };
            if about.is_some() && p.has("point") {
                return fail(format!(
                    "{ID}: `point` goes with an `axis` that is a direction, not with a datum axis: a datum axis says where it lies"
                ));
            }
            PatternLayout::Circular {
                point: triple(&p, "point", origin)?,
                // Kept, unused: the line a pattern without a datum goes
                // back to.
                axis: match about {
                    Some(_) => exprs(z_axis()),
                    None => triple(&p, "axis", z_axis)?,
                },
                angle: Expr::new(angle),
                about,
            }
        }
    };
    let kind = FeatureKind::Pattern(Pattern {
        source,
        count,
        layout,
    });
    add_feature(session, ID, p.str("name")?, kind)
}

fn feature_boolean(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.boolean";
    let p = Params::new(ID, params, &["op", "target", "tool", "keepTool", "name"])?;
    let op = p.required_str("op")?;
    let Some(op) = BooleanOp::from_name(op) else {
        return fail(format!(
            "{ID}: unknown op `{op}`; use union, subtract or intersect"
        ));
    };
    let doc = session.document()?.history.current();
    let target = feature_ref(doc, &p, ID, "target")?;
    let tool = feature_ref(doc, &p, ID, "tool")?;
    if target == tool {
        return fail(format!(
            "{ID}: `target` and `tool` are the same feature; a body cannot be combined with itself"
        ));
    }
    let kind = FeatureKind::Boolean(Boolean {
        op,
        target,
        tool,
        keep_tool: p.bool("keepTool")?.unwrap_or(false),
    });
    add_feature(session, ID, p.str("name")?, kind)
}
