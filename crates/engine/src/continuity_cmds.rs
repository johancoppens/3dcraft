//! How two faces meet along an edge, proven (`measure.continuity`, and the
//! assertion and output measure `continuity`): the order reached, G0, G1
//! or G2, and the largest gaps in position, tangent-plane angle and
//! normal curvature across the edge, each a bound over the whole edge
//! (`craft3d_geom::network::continuity`; docs/design/surfacing.md).

use crate::commands::{Run, spec};
use crate::params::Params;
use crate::verify_cmds::{Ctx, Dim, Outcome};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::PersistentName;
use craft3d_geom::Surface;
use craft3d_geom::network::{Continuity, Gap, Order, continuity};
use craft3d_topo::Body;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.continuity", "Continuity", r#"{"edge": edge name, "face": face name = the edge's other face}"#,
        "How two faces meet along an edge, proven: order is G2 (one tangent plane, and the first face's second derivatives along the other's normal matching the other's second fundamental form on the edge's tangent vectors: normal curvature across and along the edge, and twist), G1 (one tangent plane), G0 (in position) or none, the highest whose gaps are all proven within their thresholds all along the edge, not only where it was looked at. position (the distance between the faces, in the document's length unit), angle (between their tangent planes, radians), and per length unit curvature (the difference of their normal curvatures across the edge, in the direction the first face leaves it), twist (of the rate the normal turns about the edge as one goes along it) and along (of their normal curvatures along the edge), all along the second face's normal (the normals' own difference is the angle's), each give bound (proven: nothing along the edge is beyond it), found (the largest seen, at param 0 to 1 along the edge and at the point at), threshold and within. Thresholds come from the document: the tolerance, the angular tolerance, and for curvature, twist and along min(angular / L, 2 tolerance / L^2), L the edge's length: what turns the normals apart by the angular tolerance or opens a gap of the tolerance over that length; so a seam whose cross tangent turns about the edge within the angular tolerance is G1 but not G2 when that twists it beyond the curvature threshold. An order whose bounds do not come within their thresholds is not reached (nearly G2 is G1); exhausted is true when the work allowed was spent first. The edge's face, or one of the two faces, must be a spline face the edge runs along a parameter line of (a network sheet's edge is); the other a plane, cylinder, cone or sphere, or a spline face the edge runs along a parameter line of in the same parametrisation (a sheet held to that face is). Without face, the edge's two faces are compared. Refused with the reason: two analytic faces, a torus, an edge off its spline face's parameter lines. Read-only. As an assertion or output: measure continuity (edge, face) is the order as a number, -1 none, 0 G0, 1 G1, 2 G2: {\"measure\": \"continuity\", \"edge\": ..., \"min\": 2} asserts G2.", doc: true, names: ["edge", "face"], measure_continuity),
];

/// A face or an edge by name in the newest body that answers to it: the
/// body and the index.
fn locate(ctx: &Ctx<'_>, name: &PersistentName, edge: bool) -> Result<(Arc<Body>, usize), String> {
    for outcome in ctx.regen.features.iter().rev() {
        let (Some(body), Some(names)) = (outcome.own_body(), outcome.names.as_ref()) else {
            continue;
        };
        if outcome.consumed_by.is_some() {
            continue;
        }
        let found = if edge {
            names.edge_named(name)
        } else {
            (0..body.faces.len()).find(|&k| names.face_answers_to(k, name))
        };
        if let Some(k) = found {
            return Ok((Arc::clone(body), k));
        }
    }
    Err(format!(
        "no body has the {} {name} (see edgeNames and faceNames in document.inspect)",
        if edge { "edge" } else { "face" }
    ))
}

fn name_of(
    p: &Map<String, Value>,
    key: &str,
    edge: bool,
) -> Result<Option<PersistentName>, String> {
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let Value::String(text) = value else {
        return Err(format!(
            "`{key}` must be {} name",
            if edge { "an edge" } else { "a face" }
        ));
    };
    let name: PersistentName = text.parse().map_err(|e| format!("`{key}`: {e}"))?;
    if edge && !name.is_edge() {
        return Err(format!("`{key}`: {name} is not an edge"));
    }
    if !edge && !name.is_face() {
        return Err(format!("`{key}`: {name} is not a face"));
    }
    Ok(Some(name))
}

/// The measure itself.
pub(crate) fn measure(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Continuity, String> {
    crate::verify_cmds::only(p, &["edge", "face"])?;
    let edge_name = name_of(p, "edge", true)?.ok_or("give `edge`: the edge to measure along")?;
    let face_name = name_of(p, "face", false)?;
    let (body, edge) = locate(ctx, &edge_name, true)?;
    let e = body.edges.get(edge).ok_or("the edge is not in its body")?;
    let curve = craft3d_regen::edge_spline(&e.curve, e.range)
        .map_err(|why| format!("{edge_name}: {why}"))?;
    let around: Vec<usize> = craft3d_topo::edge_faces(&body)
        .get(edge)
        .map(|fs| fs.iter().map(|f| f.0 as usize).collect())
        .unwrap_or_default();
    let surface_of = |b: &Body, k: usize| -> Result<Surface, String> {
        b.faces
            .get(k)
            .map(|f| f.surface.clone())
            .ok_or_else(|| "a face is not in its body".to_string())
    };
    let (a, b) = match &face_name {
        Some(name) => {
            let (other, k) = locate(ctx, name, false)?;
            let same = Arc::ptr_eq(&other, &body);
            let own: Vec<usize> = around
                .iter()
                .copied()
                .filter(|f| !(same && *f == k))
                .collect();
            let spline = own.iter().copied().find(|f| {
                matches!(
                    body.faces.get(*f).map(|x| &x.surface),
                    Some(Surface::Nurbs(_) | Surface::Bezier(_))
                )
            });
            let first = spline
                .or_else(|| own.first().copied())
                .ok_or_else(|| format!("{edge_name} has no face of its own beside {name}"))?;
            (surface_of(&body, first)?, surface_of(&other, k)?)
        }
        None => match around.as_slice() {
            [x, y] => (surface_of(&body, *x)?, surface_of(&body, *y)?),
            _ => {
                return Err(format!(
                    "{edge_name} lies on {} face(s) of its body; give `face`: the face it meets",
                    around.len()
                ));
            }
        },
    };
    continuity(&a, &b, &curve, ctx.tolerance.linear, ctx.tolerance.angular)
        .map_err(|e| format!("{edge_name}: {e}"))
}

fn gap_json(g: &Gap) -> Value {
    json!({
        "bound": g.bound,
        "found": g.found,
        "param": g.param,
        "at": [g.at.x, g.at.y, g.at.z],
        "threshold": g.threshold,
        "within": g.within(),
    })
}

/// The measure as JSON (for its command and a network's report).
pub(crate) fn continuity_json(c: &Continuity) -> Value {
    json!({
        "order": c.order.name(),
        "position": gap_json(&c.position),
        "angle": gap_json(&c.angle),
        "curvature": gap_json(&c.curvature),
        "twist": gap_json(&c.twist),
        "along": gap_json(&c.along),
        "length": c.length,
        "pieces": c.pieces,
        "exhausted": c.exhausted,
    })
}

fn order_number(order: Order) -> f64 {
    match order {
        Order::None => -1.0,
        Order::G0 => 0.0,
        Order::G1 => 1.0,
        Order::G2 => 2.0,
    }
}

/// The assertion and output measure `continuity`: the order as a number.
/// What is proven: at least that order; at most it where the next gap was
/// seen beyond its threshold, else possibly more (not proven either way).
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    let c = measure(ctx, p)?;
    let number = order_number(c.order);
    let next_beyond = match c.order {
        Order::None => Some(c.position.beyond()),
        Order::G0 => Some(c.angle.beyond()),
        Order::G1 => Some(c.second_order().iter().any(|g| g.beyond())),
        Order::G2 => None,
    };
    let upper = match next_beyond {
        Some(false) => 2.0,
        _ => number,
    };
    let mut out = Outcome::number(Dim::Count, number);
    out.upper = Some(upper);
    if let Value::Object(detail) = continuity_json(&c) {
        out.detail = detail;
    }
    Ok(out)
}

fn measure_continuity(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.continuity";
    Params::new(ID, params, &["edge", "face"])?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let ctx = Ctx::new(doc, &regen).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let c = measure(&ctx, &map).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut result = match continuity_json(&c) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    result.insert(
        "lengthUnit".into(),
        doc.units.length.symbol().to_string().into(),
    );
    Ok(Value::Object(result))
}
