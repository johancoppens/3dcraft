//! Surfaces through curve networks (docs/design/surfacing.md): a sheet
//! through u and v curves that cross (a Gordon surface), or through four
//! curves round a patch (a Coons patch), each edge free, held on a face or
//! tangent to one.

use crate::commands::{Run, add_feature, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Document, EdgeHold, Expr, FeatureKind, MAX_NETWORK_CURVES, NetworkEdges, Rail, SurfaceNetwork,
};
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.surface.network", "Network Surface", r#"{"u": [curve, ...], "v": [curve, ...], "edges": {uFirst|uLast|vFirst|vLast: {kind: free|position|tangent, face: face name, weight: expr = 1}} = all free, "name": string = automatic, "when": expr = always on}"#,
        "Adds a sheet through a network of curves that cross (a Gordon surface): the u curves follow each other across the v curves, 2 to 64 of each. A curve is a sketch whose curves make one open chain (lines, arcs, splines end to end) or edges of earlier bodies end to end ({\"edges\": [...]}). Every u curve must cross every v curve within the tolerance; the first and last curve of each family are the sheet's edges, so the others end on them; curves are run the network's way whatever way they were drawn, and a family must cross the other in its order. The sheet passes through every curve exactly (within the gap where they cross, reported as mismatch) and is an exact B-spline: two curves each way give the Coons patch, curves along a cylinder or sphere give that surface exactly. edges holds an edge: position checks that its curve lies on the face; tangent makes the sheet meet the face with one tangent plane (G1) all along the edge, the curves crossing the edge bent to leave it along the face with their own speed times weight (a spline face along one of its parameter lines, a plane, cylinder, cone or sphere along any curve on it). The face is F9:side(eA,eB,eC,eD), its numbers the feature ids of the sketches (or of the first edges' features) of the first and last u curve and the first and last v curve; its edges F9:startEdge(0, key) and F9:endEdge(0, key) along the first and last u curve, F9:startEdge(1, key) and F9:endEdge(1, key) along the v curves. The result reports network: mismatch and the tangency found on each held edge. Refused with the curves named: curves that do not cross, cross out of order or run past an edge, rational curves whose weights cannot agree, a curve leaving a tangent edge square to its face, a corner where two held edges ask different tangent planes, tangency to a torus or along a spline face's curve that is not a parameter line. Give it a thickness with feature.plate. feature.edit changes u, v and edges.", doc: true, names: ["u", "v", "edges"], feature_network),
    spec!("feature.surface.coons", "Coons Patch", r#"{"curves": [curve, curve, curve, curve], "edges": {uFirst|uLast|vFirst|vLast: {kind: free|position|tangent, face: face name, weight: expr = 1}} = all free, "name": string = automatic, "when": expr = always on}"#,
        "Adds the Coons patch of four curves round a patch, end to end in order (each a sketch's open chain or edges of earlier bodies, as feature.surface.network takes them): the network of curves 1 and 3 as its u curves (uFirst, uLast) and curves 4 and 2 as its v curves (vFirst, vLast), so the bilinearly blended surface between them; it is a feature.surface.network and is edited as one.", doc: true, names: ["curves", "edges"], feature_coons),
];

/// A list of network curves.
pub(crate) fn curves_value(
    doc: &Document,
    p: &Params,
    command: &'static str,
    key: &str,
) -> Result<Vec<Rail>, CommandError> {
    let value = p.get(key).ok_or_else(|| p.missing(key))?;
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `{key}` must be a list of curves (sketches or {{\"edges\": [...]}})"
        ));
    };
    if items.len() < 2 || items.len() > MAX_NETWORK_CURVES {
        return fail(format!(
            "{command}: `{key}` has 2 to {MAX_NETWORK_CURVES} curves, got {}",
            items.len()
        ));
    }
    let mut out: Vec<Rail> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
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

/// The edge conditions, from `{"uFirst": {...}, ...}`.
fn edges_value(p: &Params, command: &'static str) -> Result<NetworkEdges, CommandError> {
    let mut edges = NetworkEdges::default();
    let Some(value) = p.get("edges") else {
        return Ok(edges);
    };
    let Some(map) = value.as_object() else {
        return fail(format!(
            "{command}: `edges` must be an object of uFirst, uLast, vFirst and vLast"
        ));
    };
    if let Some(other) = map
        .keys()
        .find(|k| !["uFirst", "uLast", "vFirst", "vLast"].contains(&k.as_str()))
    {
        return fail(format!(
            "{command}: `edges` has no edge `{other}`; the edges are uFirst, uLast, vFirst and vLast"
        ));
    }
    for (key, slot) in edges.all_mut() {
        if let Some(item) = map.get(key) {
            *slot = hold_value(command, key, item)?;
        }
    }
    Ok(edges)
}

fn hold_value(command: &str, key: &str, item: &Value) -> Result<EdgeHold, CommandError> {
    let Some(obj) = item.as_object() else {
        return fail(format!(
            "{command}: `edges.{key}` must be {{\"kind\": \"free|position|tangent\", \"face\": face name, \"weight\": expr}}"
        ));
    };
    if let Some(other) = obj
        .keys()
        .find(|k| !["kind", "face", "weight"].contains(&k.as_str()))
    {
        return fail(format!(
            "{command}: `edges.{key}` takes kind, face and weight, not `{other}`"
        ));
    }
    let kind = obj.get("kind").and_then(Value::as_str).unwrap_or("free");
    let face = |required: bool| -> Result<Option<craft3d_doc::PersistentName>, CommandError> {
        match obj.get("face") {
            Some(v) => Ok(Some(crate::datum_cmds::face_value(
                command,
                &format!("edges.{key}.face"),
                v,
            )?)),
            None if required => fail(format!(
                "{command}: `edges.{key}` is held to a face: give `face`"
            )),
            None => Ok(None),
        }
    };
    match kind {
        "free" => {
            if obj.contains_key("face") || obj.contains_key("weight") {
                return fail(format!(
                    "{command}: `edges.{key}` is free: it takes no face or weight"
                ));
            }
            Ok(EdgeHold::Free)
        }
        "position" => {
            if obj.contains_key("weight") {
                return fail(format!(
                    "{command}: `edges.{key}`: a weight is for a tangent edge"
                ));
            }
            let face = face(true)?.ok_or_else(|| {
                CommandError::new(format!("{command}: `edges.{key}` needs a face"))
            })?;
            Ok(EdgeHold::Position { face })
        }
        "tangent" => {
            let face = face(true)?.ok_or_else(|| {
                CommandError::new(format!("{command}: `edges.{key}` needs a face"))
            })?;
            let weight = match obj.get("weight") {
                None => Expr::new("1"),
                Some(Value::Number(n)) => {
                    let w = n.as_f64().unwrap_or(f64::NAN);
                    if !(w.is_finite() && w > 0.0) {
                        return fail(format!(
                            "{command}: `edges.{key}.weight` is a positive number, got {n}"
                        ));
                    }
                    Expr::new(n.to_string())
                }
                Some(Value::String(text)) if !text.trim().is_empty() => Expr::new(text.clone()),
                Some(_) => {
                    return fail(format!(
                        "{command}: `edges.{key}.weight` must be a number or an expression"
                    ));
                }
            };
            Ok(EdgeHold::Tangent { face, weight })
        }
        other => fail(format!(
            "{command}: `edges.{key}.kind` is free, position or tangent, not `{other}`"
        )),
    }
}

fn check_families(command: &str, u: &[Rail], v: &[Rail]) -> Result<(), CommandError> {
    if let Some(shared) = u.iter().find(|r| v.contains(r)) {
        let what = match shared {
            Rail::Sketch(id) => format!("{id}"),
            Rail::Edges(edges) => edges.first().map(ToString::to_string).unwrap_or_default(),
        };
        return fail(format!("{command}: {what} is both a u curve and a v curve"));
    }
    Ok(())
}

fn feature_network(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.network";
    let p = Params::new(ID, params, &["u", "v", "edges", "name"])?;
    let doc = session.document()?.history.current();
    let u = curves_value(doc, &p, ID, "u")?;
    let v = curves_value(doc, &p, ID, "v")?;
    check_families(ID, &u, &v)?;
    let edges = edges_value(&p, ID)?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceNetwork(SurfaceNetwork { u, v, edges }),
    )
}

fn feature_coons(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.surface.coons";
    let p = Params::new(ID, params, &["curves", "edges", "name"])?;
    let doc = session.document()?.history.current();
    let curves = curves_value(doc, &p, ID, "curves")?;
    let [a, b, c, d] = curves.as_slice() else {
        return fail(format!(
            "{ID}: `curves` are the four curves round the patch, got {}",
            curves.len()
        ));
    };
    let edges = edges_value(&p, ID)?;
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::SurfaceNetwork(SurfaceNetwork {
            u: vec![a.clone(), c.clone()],
            v: vec![d.clone(), b.clone()],
            edges,
        }),
    )
}

/// `feature.edit` for a network surface: `u`, `v`, `edges`.
pub(crate) fn edit(
    doc: &Document,
    p: &Params,
    sheet: &mut SurfaceNetwork,
) -> Result<(), CommandError> {
    if p.has("u") {
        sheet.u = curves_value(doc, p, "feature.edit", "u")?;
    }
    if p.has("v") {
        sheet.v = curves_value(doc, p, "feature.edit", "v")?;
    }
    check_families("feature.edit", &sheet.u, &sheet.v)?;
    if p.has("edges") {
        sheet.edges = edges_value(p, "feature.edit")?;
    }
    Ok(())
}

/// What a network surface reports in its feature's result.
pub(crate) fn report_json(map: &mut Map<String, Value>, made: &craft3d_regen::Networked) {
    let mut tangency = Map::new();
    for (key, value) in ["uFirst", "uLast", "vFirst", "vLast"]
        .iter()
        .zip(made.tangency)
    {
        if let Some(angle) = value {
            tangency.insert((*key).to_string(), json!(angle));
        }
    }
    map.insert(
        "network".into(),
        json!({"mismatch": made.mismatch, "tangency": tangency}),
    );
}
