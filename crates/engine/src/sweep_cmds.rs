//! Sweeps of a sketch's profiles other than the straight extrusion
//! (`feature.extrude` is in commands.rs): the revolution.

use crate::commands::{Run, add_sweep, combine_param, doc_error, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::{Document, EntityId, Expr, FeatureId, FeatureKind, Revolve, RevolveAxis};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.revolve", "Revolve", r#"{"sketch": id or name, "axis": "x" | "y" | line id | datum axis, "angle": expr = "360 deg", "symmetric": boolean = false, "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic}"#,
        "Adds a solid by turning the closed profiles of a sketch about an axis in the sketch's plane: its x or y axis, one of its lines (\"e7\"; a construction line does not become part of the profile), or a datum axis that lies in the sketch's plane (its name or id; the solid follows the datum). The profiles may touch the axis but not cross it. angle is counter-clockwise seen from the tip of the axis (for a line: from its start to its end; for a datum axis: along its direction); a negative angle goes the other way, symmetric puts half on each side of the sketch plane, and 360 deg closes the body. cut, join and intersect combine the solid with a body at once, as for feature.extrude.", doc: true, feature_revolve),
];

/// `"x"`, `"y"`, a line of the sketch as `"e7"` or `7`, or a datum axis
/// by its name or id (`"F3"`). The sketch's own axes and lines win over a
/// feature of the same name.
pub(crate) fn revolve_axis(
    doc: &Document,
    command: &str,
    value: &Value,
) -> Result<RevolveAxis, crate::CommandError> {
    let wrong = || {
        fail(format!(
            "{command}: `axis` must be \"x\", \"y\", a line of the sketch such as \"e7\", or a datum axis"
        ))
    };
    let datum = |text: &str| {
        let feature = doc.feature_by_name(text).or_else(|| {
            let id = text.strip_prefix('F')?.parse().ok()?;
            doc.feature(FeatureId(id))
        })?;
        Some(match feature.kind {
            FeatureKind::DatumAxis(_) => Ok(RevolveAxis::Datum { axis: feature.id }),
            _ => fail(format!(
                "{command}: `axis`: `{}` is a {}, not a datum axis",
                feature.name,
                feature.kind.type_name()
            )),
        })
    };
    match value {
        Value::String(text) => match text.as_str() {
            "x" | "X" => Ok(RevolveAxis::X),
            "y" | "Y" => Ok(RevolveAxis::Y),
            other => match other.strip_prefix('e').and_then(|d| d.parse::<u64>().ok()) {
                Some(id) => Ok(RevolveAxis::Line {
                    entity: EntityId(id),
                }),
                None => datum(other).unwrap_or_else(wrong),
            },
        },
        Value::Number(n) => match n.as_u64() {
            Some(id) => Ok(RevolveAxis::Line {
                entity: EntityId(id),
            }),
            None => wrong(),
        },
        _ => wrong(),
    }
}

fn feature_revolve(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.revolve";
    let p = Params::new(
        ID,
        params,
        &[
            "sketch",
            "axis",
            "angle",
            "symmetric",
            "cut",
            "join",
            "intersect",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    doc.profile_source(sketch).map_err(doc_error(ID))?;
    let axis = revolve_axis(doc, ID, p.get("axis").ok_or_else(|| p.missing("axis"))?)?;
    let combine = combine_param(doc, &p, ID)?;
    let kind = FeatureKind::Revolve(Revolve {
        sketch,
        axis,
        angle: Expr::new(p.expr("angle")?.unwrap_or_else(|| "360 deg".to_string())),
        symmetric: p.bool("symmetric")?.unwrap_or(false),
    });
    add_sweep(session, ID, p.str("name")?, kind, combine)
}
