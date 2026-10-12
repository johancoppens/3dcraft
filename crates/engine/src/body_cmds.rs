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
use craft3d_doc::{Expr, FeatureKind, Mirror, Pattern, PatternLayout, Transform};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.transform", "Move Body", r#"{"source": id or name, "translate": [expr, expr, expr] = [0, 0, 0], "angle": expr = "0 deg", "axis": [number, number, number] = [0, 0, 1], "pivot": [expr, expr, expr] = [0, 0, 0], "keep": boolean = false, "name": string = automatic}"#,
        "Adds a feature whose body is the body of source, turned by angle about the line through pivot along axis (counter-clockwise seen from the tip of axis) and then shifted by translate. The source's body goes into it; with keep the source keeps its body and this is a moved copy. Faces keep their names with a prefix: \"F5:copy(0)/F2:side(e7)\".", doc: true, feature_transform),
    spec!("feature.mirror", "Mirror Body", r#"{"source": id or name, "normal": [number, number, number], "point": [expr, expr, expr] = [0, 0, 0], "keep": boolean = false, "name": string = automatic}"#,
        "Adds a feature whose body is the body of source mirrored in the plane through point with the given normal. With keep the source keeps its body and this is a mirrored copy.", doc: true, feature_mirror),
    spec!("feature.pattern", "Pattern", r#"{"source": id or name, "count": expr, "step": [expr, expr, expr], "angle": expr, "axis": [number, number, number] = [0, 0, 1], "point": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a feature whose body is count instances of the body of source, the source included. Linear: give step, the shift from one instance to the next. Circular: give angle, the turn from one instance to the next about the line through point along axis (\"360 deg / n\" with count n goes evenly round). count is a whole number and may be a formula of variables. The instances must not overlap each other. The faces of instance i are named \"F5:copy(i)/...\".", doc: true, feature_pattern),
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
    let p = Params::new(ID, params, &["source", "normal", "point", "keep", "name"])?;
    let source = feature_ref(session.document()?.history.current(), &p, ID, "source")?;
    let normal = p.expr3("normal")?.ok_or_else(|| p.missing("normal"))?;
    let kind = FeatureKind::Mirror(Mirror {
        source,
        point: triple(&p, "point", origin)?,
        normal: exprs(normal),
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
    let source = feature_ref(session.document()?.history.current(), &p, ID, "source")?;
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
        (None, Some(angle)) => PatternLayout::Circular {
            point: triple(&p, "point", origin)?,
            axis: triple(&p, "axis", z_axis)?,
            angle: Expr::new(angle),
        },
    };
    let kind = FeatureKind::Pattern(Pattern {
        source,
        count,
        layout,
    });
    add_feature(session, ID, p.str("name")?, kind)
}
