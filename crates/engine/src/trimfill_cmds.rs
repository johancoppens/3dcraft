//! Trimmed faces and N-sided fills (docs/design/surfacing.md › *Trimmed
//! faces and N-sided fills*): a face on the surface of an earlier face,
//! bounded by loops of curves taken onto it and made exact there; and a
//! smooth patch inside a closed loop of three to six curves.

use crate::commands::{Run, add_feature, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Document, EdgeHold, Expr, FeatureKind, InnerSeams, MAX_NETWORK_CURVES, MAX_NSIDED_CURVES,
    MAX_TRIM_LOOPS, Rail, SurfaceNsided, SurfaceTrim, TrimSpace,
};
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.surface.trim", "Trimmed Face", r#"{"face": face name, "outer": [curve, ...], "holes": [[curve, ...], ...] = none, "space": model|parameter = model, "along": [x, y, z] = closest points, "name": string = automatic, "when": expr = always on}"#,
        "Adds a sheet of one face on the surface of `face` (a face of an earlier body: a spline surface, a plane, a cylinder, cone, sphere or torus), the surface itself, exactly, bounded by `outer` and cut by `holes`. Each loop is 1 to 64 curves end to end that close (a sketch's open chain or closed loop, or edges of earlier bodies as {\"edges\": [...]}); a curve may be given alone instead of a list. In model space each curve is taken onto the surface, to its closest points or along the direction `along`, and its edge is the curve on the surface through the parameters of its projection (sampled until it lies within half the tolerance of the projection, proven at every point between the samples, not only at them; the proven bound is reported as deviation, the largest gap found at the midpoints as sampled). In parameter space each curve is a sketch whose x and y are read as the surface's u and v in percent of its domain (0 to 100 each way). On a spline surface every edge is S(c(t)) made exactly (degree d(p + q), refused above 32); on a plane the projected curve itself. Loops are turned to bound the face (the outer one counter-clockwise about its normal); the face keeps the source face's side. On a cylinder, cone, sphere or torus the edges are made exact on its rational form (the face stays analytic); a loop that goes round the axis is refused. The face is F9:face(0); an edge F9:startEdge(k, s) after its curve (k the sketch's id or the feature of the first edge, s the sketch curve or the edge's place); a vertex F9:startVertex(k1, s1, k2, s2) after the two curves it joins, the pairs in ascending order. Refused with the curve named: loops that do not close, a curve that leaves the surface or whose line along `along` misses it, a loop enclosing nothing, a loop round an analytic face's axis. feature.edit changes face, outer, holes, space and along.", doc: true, names: ["face", "outer", "holes"], feature_trim),
    spec!("feature.surface.nsided", "N-Sided Fill", r#"{"curves": [curve, ...], "edges": [{kind: free|position|tangent|curvature, face: face name, weight: expr = 1}, ...] = all free, "inner": tangent|curvature = tangent, "rise": expr = 0, "name": string = automatic, "when": expr = always on}"#,
        "Adds a smooth sheet inside a closed loop of 3 to 6 curves end to end (each a sketch's open chain or edges of earlier bodies, as a network's curves), made of one four-sided patch per curve around a centre point (Hahn's split: every corner regular, no pole), each patch an exact biquintic B-spline. edges holds each curve's edge (one entry per curve, in order): position checks the curve lies on the face; tangent makes the fill meet the face with one tangent plane (G1) all along, curvature also with its curvature (G2) (a plane, cylinder, cone or sphere along any curve on it, a spline face along one of its parameter lines). inner says how the patches meet across the spokes from the centre to each curve's midpoint: tangent (G1) or curvature (G2). Every seam and held curve is proven by measure.continuity; where one asked in curvature is not proven G2 the fill is built again with every curvature hold tangent and each seam and curve says what was asked, what was built and why. rise moves the centre along the fill's normal (the side its curves run counter-clockwise about) by that length: a dome, or with a negative rise a scoop. A hold's weight is not used. The faces are F9:side(k1, k2) after the two curves at their outer corner (k a sketch's id, or a hash of the first edge's name), the halves of a curve F9:startEdge(k) and F9:endEdge(k), the spoke to it F9:lateralEdge(k), its midpoint F9:startVertex(k), the corner between two curves F9:endVertex(k1, k2), the centre F9:startVertex(0). The result reports fill: per seam what was asked and built, why less, and its continuity; per held curve the continuity of both halves. Refused with the curve named: loops that do not close, fewer than 3 or more than 6 curves, a curve off its position face, a curve held to a torus, a spoke whose shared plane turns along it, a seam not proven G1. feature.edit changes curves, edges, inner and rise.", doc: true, names: ["curves", "edges"], feature_nsided),
];

/// A loop: a list of curves, or one curve alone.
fn loop_value(
    doc: &Document,
    command: &'static str,
    key: &str,
    value: &Value,
) -> Result<Vec<Rail>, CommandError> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    if items.is_empty() || items.len() > MAX_NETWORK_CURVES {
        return fail(format!(
            "{command}: `{key}` is a loop of 1 to {MAX_NETWORK_CURVES} curves, got {}",
            items.len()
        ));
    }
    let mut out: Vec<Rail> = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let slot = format!("{key}[{i}]");
        let wrapped = json!({ "curve": item });
        let one = Params::new(command, &wrapped, &["curve"])?;
        let rail = crate::develop_cmds::rail_value(doc, &one, command, "curve").map_err(|e| {
            CommandError::new(e.to_string().replace("`curve`", &format!("`{slot}`")))
        })?;
        if let Some(again) = out.iter().position(|r| *r == rail) {
            return fail(format!(
                "{command}: `{slot}` is the same curve as `{key}[{again}]`"
            ));
        }
        out.push(rail);
    }
    Ok(out)
}

fn holes_value(
    doc: &Document,
    command: &'static str,
    value: &Value,
) -> Result<Vec<Vec<Rail>>, CommandError> {
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `holes` must be a list of loops (each a list of curves)"
        ));
    };
    if items.len() >= MAX_TRIM_LOOPS {
        return fail(format!(
            "{command}: `holes` has at most {} loops, got {}",
            MAX_TRIM_LOOPS - 1,
            items.len()
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| loop_value(doc, command, &format!("holes[{i}]"), item))
        .collect()
}

fn space_value(command: &str, value: &Value) -> Result<TrimSpace, CommandError> {
    match value.as_str() {
        Some("model") => Ok(TrimSpace::Model),
        Some("parameter") => Ok(TrimSpace::Parameter),
        _ => fail(format!(
            "{command}: `space` is \"model\" or \"parameter\", got {value}"
        )),
    }
}

fn along_value(p: &Params) -> Result<Option<[Expr; 3]>, CommandError> {
    if matches!(p.get("along"), None | Some(Value::Null)) {
        return Ok(None);
    }
    let Some(three) = p.expr3("along")? else {
        return Ok(None);
    };
    Ok(Some(three.map(Expr::new)))
}

/// Every loop of the face: no curve in two of them.
fn check_loops(command: &str, outer: &[Rail], holes: &[Vec<Rail>]) -> Result<(), CommandError> {
    let all: Vec<&Rail> = outer.iter().chain(holes.iter().flatten()).collect();
    for (i, a) in all.iter().enumerate() {
        if all.iter().skip(i + 1).any(|b| b == a) {
            return fail(format!("{command}: one curve is in two loops"));
        }
    }
    Ok(())
}

fn feature_trim(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.trim";
    let p = Params::new(
        ID,
        params,
        &["face", "outer", "holes", "space", "along", "name"],
    )?;
    let doc = session.document()?.history.current();
    let face =
        crate::datum_cmds::face_value(ID, "face", p.get("face").ok_or_else(|| p.missing("face"))?)?;
    let outer = loop_value(
        doc,
        ID,
        "outer",
        p.get("outer").ok_or_else(|| p.missing("outer"))?,
    )?;
    let holes = match p.get("holes") {
        Some(v) => holes_value(doc, ID, v)?,
        None => Vec::new(),
    };
    check_loops(ID, &outer, &holes)?;
    let space = match p.get("space") {
        Some(v) => space_value(ID, v)?,
        None => TrimSpace::Model,
    };
    let along = along_value(&p)?;
    if along.is_some() && space == TrimSpace::Parameter {
        return fail(format!(
            "{ID}: `along` projects model-space curves; curves in parameter space are not projected"
        ));
    }
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceTrim(SurfaceTrim {
            face,
            outer,
            holes,
            space,
            along,
        }),
    )
}

/// `feature.edit` for a trimmed face: `face`, `outer`, `holes`, `space`,
/// `along`.
pub(crate) fn edit_trim(
    doc: &Document,
    p: &Params,
    trim: &mut SurfaceTrim,
) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    if let Some(v) = p.get("face") {
        trim.face = crate::datum_cmds::face_value(ID, "face", v)?;
    }
    if let Some(v) = p.get("outer") {
        trim.outer = loop_value(doc, ID, "outer", v)?;
    }
    if let Some(v) = p.get("holes") {
        trim.holes = holes_value(doc, ID, v)?;
    }
    if let Some(v) = p.get("space") {
        trim.space = space_value(ID, v)?;
    }
    if p.has("along") {
        trim.along = along_value(p)?;
    }
    check_loops(ID, &trim.outer, &trim.holes)?;
    if trim.along.is_some() && trim.space == TrimSpace::Parameter {
        return fail(format!(
            "{ID}: `along` projects model-space curves; curves in parameter space are not projected"
        ));
    }
    Ok(())
}

/// What a trimmed face reports in its feature's result.
pub(crate) fn trim_json(map: &mut Map<String, Value>, made: &craft3d_regen::Trimmed) {
    map.insert(
        "trim".into(),
        json!({"deviation": made.deviation, "sampled": made.sampled, "degree": made.degree, "loops": made.loops}),
    );
}

fn holds_value(
    p: &Params,
    command: &'static str,
    count: usize,
) -> Result<Vec<EdgeHold>, CommandError> {
    let Some(value) = p.get("edges") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `edges` is a list of holds, one per curve ({{\"kind\": \"free|position|tangent|curvature\", \"face\": face name}})"
        ));
    };
    if items.len() != count {
        return fail(format!(
            "{command}: `edges` has one hold per curve ({count}), got {}",
            items.len()
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| crate::surface_cmds::hold_value(command, &format!("{i}"), item))
        .collect()
}

fn inner_value(command: &str, value: &Value) -> Result<InnerSeams, CommandError> {
    match value.as_str() {
        Some("tangent") => Ok(InnerSeams::Tangent),
        Some("curvature") => Ok(InnerSeams::Curvature),
        _ => fail(format!(
            "{command}: `inner` is \"tangent\" or \"curvature\", got {value}"
        )),
    }
}

fn nsided_curves(
    doc: &Document,
    p: &Params,
    command: &'static str,
) -> Result<Vec<Rail>, CommandError> {
    let value = p.get("curves").ok_or_else(|| p.missing("curves"))?;
    let curves = loop_value(doc, command, "curves", value)?;
    if !(3..=MAX_NSIDED_CURVES).contains(&curves.len()) {
        return fail(format!(
            "{command}: `curves` are 3 to {MAX_NSIDED_CURVES} curves round the fill, got {}",
            curves.len()
        ));
    }
    Ok(curves)
}

fn feature_nsided(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.nsided";
    let p = Params::new(ID, params, &["curves", "edges", "inner", "rise", "name"])?;
    let doc = session.document()?.history.current();
    let curves = nsided_curves(doc, &p, ID)?;
    let edges = holds_value(&p, ID, curves.len())?;
    let inner = match p.get("inner") {
        Some(v) => inner_value(ID, v)?,
        None => InnerSeams::Tangent,
    };
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceNsided(SurfaceNsided {
            curves,
            edges,
            inner,
            rise: p.expr("rise")?.map(Expr::new),
        }),
    )
}

/// `feature.edit` for an N-sided fill: `curves`, `edges`, `inner`. New
/// curves of another count drop the holds unless `edges` comes too.
pub(crate) fn edit_nsided(
    doc: &Document,
    p: &Params,
    fill: &mut SurfaceNsided,
) -> Result<(), CommandError> {
    const ID: &str = "feature.edit";
    if p.has("curves") {
        fill.curves = nsided_curves(doc, p, ID)?;
        if fill.edges.len() != fill.curves.len() && !p.has("edges") {
            fill.edges.clear();
        }
    }
    if p.has("edges") {
        fill.edges = holds_value(p, ID, fill.curves.len())?;
    }
    if let Some(v) = p.get("inner") {
        fill.inner = inner_value(ID, v)?;
    }
    if p.has("rise") {
        fill.rise = p.expr("rise")?.map(Expr::new);
    }
    Ok(())
}

/// What an N-sided fill reports in its feature's result.
pub(crate) fn fill_json(map: &mut Map<String, Value>, made: &craft3d_regen::Filled) {
    let name = |curvature: bool| if curvature { "curvature" } else { "tangent" };
    let seams: Vec<Value> = made
        .seams
        .iter()
        .map(|s| {
            json!({
                "asked": name(s.asked_curvature),
                "built": name(s.built_curvature),
                "why": s.why,
                "continuity": crate::continuity_cmds::continuity_json(&s.continuity),
            })
        })
        .collect();
    let outer: Vec<Value> = made
        .outer
        .iter()
        .map(|o| match o {
            Some(h) => json!({
                "asked": name(h.asked_curvature),
                "built": name(h.built_curvature),
                "why": h.why,
                "continuity": h.halves.iter().map(crate::continuity_cmds::continuity_json).collect::<Vec<Value>>(),
            }),
            None => Value::Null,
        })
        .collect();
    map.insert("fill".into(), json!({"seams": seams, "edges": outer}));
}
