//! Network surfaces in regeneration (docs/design/surfacing.md): each curve
//! of the network from a sketch's open chain or from edges of earlier
//! bodies (as a developable's rails), joined into one spline; the faces the
//! edges are held to from earlier bodies; the sheet from
//! `craft3d_geom::network::gordon`, made into a body of one face.

use crate::datum::named_body_of;
use crate::developable::rail_of;
use crate::{FeatureResult, Names};
use craft3d_doc::{
    Document, EdgeHold, FeatureId, NameRole, PersistentName, Rail as DocRail, SurfaceNetwork,
};
use craft3d_expr::Evaluation;
use craft3d_geom::network::{self, Condition, EDGE_NAMES, Network};
use craft3d_geom::refine::{piece, reversed};
use craft3d_geom::{Curve3, NurbsCurve, Surface};
use craft3d_kernel::developable::Rail;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, VertexId};

/// What a network surface made beside its body, for measures and reports.
#[derive(Clone, Debug, PartialEq)]
pub struct Networked {
    /// The largest distance between two curves where they cross.
    pub mismatch: f64,
    /// Per edge held tangent: the largest angle found between the tangent
    /// planes along it.
    pub tangency: [Option<f64>; 4],
    /// Per edge held in curvature: how the sheet meets its face there,
    /// proven (`network::continuity`).
    pub seams: Box<[Option<network::Continuity>; 4]>,
}

/// The curve of an edge over its range as a spline (lines and arcs
/// exactly), for measures along it.
pub fn edge_spline(curve: &Curve3, range: (f64, f64)) -> Result<NurbsCurve, String> {
    let (a, b) = range;
    match curve {
        Curve3::Line(_) => NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![curve.eval(a), curve.eval(b)],
        ),
        Curve3::Circle(circle) => network::arc(circle.frame(), circle.radius(), a, b - a),
        Curve3::Nurbs(spline) => {
            let (lo, hi) = spline.domain();
            if a <= lo && b >= hi {
                Ok(spline.clone())
            } else {
                piece(spline, a.max(lo), b.min(hi))
            }
        }
        Curve3::Ellipse(_) => {
            return Err("an ellipse is not measured along yet".to_string());
        }
    }
    .map_err(|e| e.to_string())
}

/// One spline of a rail, its pieces joined end to end.
fn spline_of(rail: &Rail, role: &str, tol: &Tolerance) -> Result<NurbsCurve, String> {
    let mut pieces = Vec::with_capacity(rail.pieces.len());
    for p in &rail.pieces {
        let (a, b) = p.range;
        let curve = match &p.curve {
            Curve3::Line(_) => NurbsCurve::polynomial(
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![p.curve.eval(a), p.curve.eval(b)],
            ),
            Curve3::Circle(circle) => network::arc(circle.frame(), circle.radius(), a, b - a),
            Curve3::Nurbs(spline) => {
                let (lo, hi) = spline.domain();
                if a <= lo && b >= hi {
                    Ok(spline.clone())
                } else {
                    piece(spline, a.max(lo), b.min(hi))
                }
            }
            Curve3::Ellipse(_) => {
                return Err(format!(
                    "{role}: an ellipse as a curve of a network is not supported yet"
                ));
            }
        }
        .map_err(|e| format!("{role}: {e}"))?;
        let curve = if p.reversed {
            reversed(&curve).map_err(|e| format!("{role}: {e}"))?
        } else {
            curve
        };
        pieces.push(curve);
    }
    network::chain(&pieces, tol.linear, tol.angular).map_err(|e| format!("{role}: {e}"))
}

/// The number a curve of the network is known by in names: its sketch's
/// id, or the id of the feature of its first edge.
fn key(rail: &DocRail) -> u64 {
    match rail {
        DocRail::Sketch(id) => id.0,
        DocRail::Edges(edges) => edges.first().map_or(0, |e| e.feature.0),
    }
}

/// The surface of the face `name` of an earlier body.
fn face_surface(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Surface, String> {
    let (body, names, made_by) = named_body_of(doc, earlier, name, None, role)?;
    let index = (0..body.faces.len())
        .find(|&i| names.face_answers_to(i, name))
        .ok_or_else(|| format!("{role}: `{made_by}` has no face {name}"))?;
    body.faces
        .get(index)
        .map(|f| f.surface.clone())
        .ok_or_else(|| format!("{role}: `{made_by}` has no face {name}"))
}

fn ratio(variables: &Evaluation, name: &str, expr: &craft3d_doc::Expr) -> Result<f64, String> {
    let quantity = variables
        .evaluate(expr.source())
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if !quantity.dimension.is_dimensionless() {
        return Err(format!(
            "{name}: `{}` is not a plain number (a tangent's weight has no unit)",
            expr.source()
        ));
    }
    if !(quantity.value.is_finite() && quantity.value > 0.0) {
        return Err(format!(
            "{name}: `{}` is {}; a tangent's weight is a positive number",
            expr.source(),
            quantity.value
        ));
    }
    Ok(quantity.value)
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    feature: &SurfaceNetwork,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Networked), String> {
    let tol = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let (m, n) = (feature.u.len(), feature.v.len());
    let most = craft3d_doc::MAX_NETWORK_CURVES;
    if !(2..=most).contains(&m) || !(2..=most).contains(&n) {
        return Err(format!(
            "u, v: a network has 2 to {most} curves each way; this one has {m} u curves and {n} v curves"
        ));
    }
    let mut us = Vec::with_capacity(m);
    for (i, rail) in feature.u.iter().enumerate() {
        let role = format!("u: curve {}", i + 1);
        let made = rail_of(doc, earlier, rail, &role, tol.linear)?;
        us.push(spline_of(&made, &role, &tol)?);
    }
    let mut vs = Vec::with_capacity(n);
    for (j, rail) in feature.v.iter().enumerate() {
        let role = format!("v: curve {}", j + 1);
        let made = rail_of(doc, earlier, rail, &role, tol.linear)?;
        vs.push(spline_of(&made, &role, &tol)?);
    }
    // The faces held to, then the conditions that borrow them.
    let holds = feature.edges.all();
    // Each edge's face, with its weight and whether held in curvature.
    type Held = Option<(Surface, Option<(f64, bool)>)>;
    let mut faces: Vec<Held> = Vec::with_capacity(4);
    for (key, hold) in &holds {
        let role = format!("edges.{key}");
        faces.push(match hold {
            EdgeHold::Free => None,
            EdgeHold::Position { face } => Some((face_surface(doc, earlier, face, &role)?, None)),
            EdgeHold::Tangent { face, weight } => Some((
                face_surface(doc, earlier, face, &role)?,
                Some((ratio(variables, &format!("{role}.weight"), weight)?, false)),
            )),
            EdgeHold::Curvature { face, weight } => Some((
                face_surface(doc, earlier, face, &role)?,
                Some((ratio(variables, &format!("{role}.weight"), weight)?, true)),
            )),
        });
    }
    let condition = |k: usize| -> Condition<'_> {
        match faces.get(k) {
            Some(Some((surface, None))) => Condition::Position(surface),
            Some(Some((surface, Some((weight, false))))) => Condition::Tangent {
                surface,
                weight: *weight,
            },
            Some(Some((surface, Some((weight, true))))) => Condition::Curvature {
                surface,
                weight: *weight,
            },
            _ => Condition::Free,
        }
    };
    let across = feature
        .across_degree
        .map_or(network::DEFAULT_ACROSS_DEGREE, usize::from);
    let made = network::gordon_across(
        &Network {
            u: &us,
            v: &vs,
            edges: [condition(0), condition(1), condition(2), condition(3)],
            tolerance: tol.linear,
            angular: tol.angular,
        },
        across,
    )
    .map_err(|e| {
        // The geometry numbers curves from 1 as the feature lists them.
        let text = e.to_string();
        let named = EDGE_NAMES
            .iter()
            .zip(holds.iter())
            .fold(text, |t, (words, (key, _))| {
                t.replace(words, &format!("{words} (edges.{key})"))
            });
        format!("u, v: {named}")
    })?;

    let keys = [
        feature.u.first().map_or(0, key),
        feature.u.last().map_or(0, key),
        feature.v.first().map_or(0, key),
        feature.v.last().map_or(0, key),
    ];
    let (body, edges, vertices) = sheet(&made.surface, &made.boundary, &keys, id, tol.linear)?;
    let face = PersistentName::new(id, NameRole::Side, keys.to_vec());
    let names = Names::listed(vec![face], edges, vertices);
    Ok((
        body,
        names,
        Networked {
            mismatch: made.mismatch,
            tangency: made.tangency,
            seams: Box::new(made.seams),
        },
    ))
}

/// The body of one face on `surface` bounded by its iso-curves `boundary`
/// (v = 0, v = 1, u = 0, u = 1; `None` where a point), with the names of
/// its edges and vertices: an edge `startEdge(family, key)` for the first
/// curve of a family (0 for u, 1 for v) and `endEdge(family, key)` for the
/// last; a corner on the first u curve `startVertex(end, key u, key v)`,
/// one on the last `endVertex(end, key u, key v)`, `end` 0 for the first
/// v curve and 1 for the last.
fn sheet(
    surface: &craft3d_geom::NurbsSurface,
    boundary: &[Option<NurbsCurve>; 4],
    keys: &[u64; 4],
    id: FeatureId,
    tol: f64,
) -> Result<(Body, Vec<PersistentName>, Vec<PersistentName>), String> {
    let mut body = Body::new(BodyKind::Sheet);
    let corner = |u: f64, v: f64| surface.eval(u, v);
    // Corners in the order (u first, v first), (u first, v last),
    // (u last, v first), (u last, v last).
    let points: [Vec3; 4] = [
        corner(0.0, 0.0),
        corner(1.0, 0.0),
        corner(0.0, 1.0),
        corner(1.0, 1.0),
    ];
    let roles = [
        (NameRole::StartVertex, 0, keys[0], keys[2]),
        (NameRole::StartVertex, 1, keys[0], keys[3]),
        (NameRole::EndVertex, 0, keys[1], keys[2]),
        (NameRole::EndVertex, 1, keys[1], keys[3]),
    ];
    let mut ids: Vec<VertexId> = Vec::with_capacity(4);
    let mut vertices = Vec::with_capacity(4);
    for (p, (role, end, ku, kv)) in points.iter().zip(roles) {
        let same = ids
            .iter()
            .zip(&points)
            .find(|(_, q)| q.distance(*p) <= tol)
            .map(|(v, _)| *v);
        ids.push(match same {
            Some(v) => v,
            None => {
                vertices.push(PersistentName::new(id, role, vec![end, ku, kv]));
                body.add_vertex(*p)
            }
        });
    }
    let (Some(&c00), Some(&c10), Some(&c01), Some(&c11)) =
        (ids.first(), ids.get(1), ids.get(2), ids.get(3))
    else {
        return Err("a network sheet lost its corners; this is a bug".to_string());
    };
    // Counter-clockwise in the surface's parameters: v = 0 forward, u = 1
    // forward, v = 1 back, u = 0 back.
    let order = [
        (0, c00, c10, false),
        (3, c10, c11, false),
        (1, c01, c11, true),
        (2, c00, c01, true),
    ];
    let mut coedges = Vec::with_capacity(4);
    let mut edges = Vec::with_capacity(4);
    for (k, start, end, back) in order {
        let Some(Some(curve)) = boundary.get(k) else {
            continue;
        };
        let edge = body.add_edge(Curve3::Nurbs(curve.clone()), curve.domain(), start, end);
        coedges.push(Coedge {
            edge,
            reversed: back,
        });
        let (role, family) = match k {
            0 => (NameRole::StartEdge, 0),
            1 => (NameRole::EndEdge, 0),
            2 => (NameRole::StartEdge, 1),
            _ => (NameRole::EndEdge, 1),
        };
        edges.push(PersistentName::new(
            id,
            role,
            vec![family, keys.get(k).copied().unwrap_or(0)],
        ));
    }
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Nurbs(surface.clone()), false, vec![lp]);
    body.add_shell(vec![face]);
    Ok((body, edges, vertices))
}
