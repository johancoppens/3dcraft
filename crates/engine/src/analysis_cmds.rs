//! The measures of surface analysis as commands (docs/design/surfacing.md,
//! stage 1): `measure.draft` and `measure.curvature`. The same measures are
//! `draft` and `curvature` of assertions and outputs.

use crate::commands::{Run, spec};
use crate::params::Params;
use crate::verify_cmds::Ctx;
use crate::{CommandError, CommandSpec, Session};
use serde_json::{Map, Value};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.draft", "Draft", r#"{"faces": subject = all bodies, "direction": "x|y|z|-x|-y|-z" or [x, y, z] = "z", "minDraft": degrees = 1}"#,
        "The draft of faces against a pull direction, for moulding: the draft angle at a point is the angle between the face and the pull, asin(n · d) with n the outward normal (positive where the face releases along the pull, negative against it). Returns draft (the least, in degrees) with where (at, face), shareBelow (the share of the faces' area whose draft is below minDraft), proven, and per face its least, its share below and how it was found: exact for a plane; on a cylinder, sphere or torus at the vertices of a fine mesh (the exact normals there) with a proven bound below that takes off the most the normal turns between them; sampled on a cone or a spline face. Read-only. As assertions: measure draft (min, in angle units: \"1 deg\").", doc: true, names: ["faces"], measure_draft),
    spec!("measure.curvature", "Curvature At", r#"{"of": subject = all bodies, "at": [x, y, z]}"#,
        "The curvature of the faces at the point of them nearest to at, from the exact surface (its second derivatives): gaussian (K, 1/length^2), mean (H, 1/length, positive where the body is convex), k1 and k2 (the principal curvatures, k1 >= k2), the face, the point and its distance from at. Read-only. As assertions: measure curvature with at (eq, min, max of K).", doc: true, names: ["of"], measure_curvature),
];

fn run(
    session: &mut Session,
    params: &Value,
    id: &'static str,
    measure: &str,
    keys: &'static [&'static str],
) -> Run {
    Params::new(id, params, keys)?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let ctx = Ctx::new(doc, &regen).map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    let out = crate::verify_cmds::take(&ctx, measure, &map)
        .map_err(|e| CommandError::new(format!("{id}: {e}")))?;
    let mut result = out.detail;
    if measure == "draft" {
        result.insert(
            "draft".into(),
            result.get("least").cloned().unwrap_or(Value::Null),
        );
    }
    result.insert(
        "lengthUnit".into(),
        doc.units.length.symbol().to_string().into(),
    );
    Ok(Value::Object(result))
}

fn measure_draft(session: &mut Session, params: &Value) -> Run {
    run(
        session,
        params,
        "measure.draft",
        "draft",
        &["faces", "direction", "minDraft"],
    )
}

fn measure_curvature(session: &mut Session, params: &Value) -> Run {
    if params.get("at").is_none() {
        return Err(CommandError::new(
            "measure.curvature: `at` is required: [x, y, z]",
        ));
    }
    run(
        session,
        params,
        "measure.curvature",
        "curvature",
        &["of", "at"],
    )
}
