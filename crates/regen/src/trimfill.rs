//! Trimmed faces and N-sided fills in regeneration
//! (docs/design/surfacing.md › *Trimmed faces and N-sided fills*): the
//! loops of a trimmed face from the history, taken onto the source face's
//! surface and made exact there (`craft3d_geom::trim`), and the sheet body
//! of one face on that very surface; the N-sided fill's curves, holds and
//! patches (`craft3d_geom::network::nsided`) as a sheet of N faces.

use crate::datum::named_body_of;
use crate::developable::rail_of;
use crate::network::edge_spline;
use crate::{FeatureResult, Names};
use craft3d_doc::{
    Document, EdgeHold, Expr, FeatureId, InnerSeams, NameRole, PersistentName, Rail as DocRail,
    SurfaceNsided, SurfaceTrim, TrimSpace,
};
use craft3d_expr::Evaluation;
use craft3d_geom::refine::reversed;
use craft3d_geom::trim::{self, Projection};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, VertexId};

/// What a trimmed face made beside its body.
#[derive(Clone, Debug, PartialEq)]
pub struct Trimmed {
    /// The largest distance found between an edge and the projection of
    /// its curve (at the midpoints between samples); 0 for curves given
    /// in parameter space or projected onto a plane, which are exact.
    pub deviation: f64,
    /// The highest degree of an edge (`d (p + q)` on a spline surface).
    pub degree: usize,
    /// Edges per loop, outer first.
    pub loops: Vec<usize>,
}

/// One edge of a loop: its curve in space, its parameter-space curve on a
/// spline surface, and what names it.
struct Piece {
    curve: NurbsCurve,
    key: u64,
    source: u64,
}

/// A loop's pieces, and its points in the surface's parameters.
type TrimLoop = (Vec<Piece>, Vec<(f64, f64)>);

/// The number a curve is known by in names: its sketch's id, or the id of
/// the feature of its first edge.
pub(crate) fn key(rail: &DocRail) -> u64 {
    match rail {
        DocRail::Sketch(id) => id.0,
        // The FNV-1a hash of the first edge's name, in 48 bits (as a rim's
        // name has it): two curves of edges of one body differ.
        DocRail::Edges(edges) => edges.first().map_or(0, |e| {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in e.to_string().bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash & 0xffff_ffff_ffff
        }),
    }
}

/// A direction from three expressions, unit.
fn direction(variables: &Evaluation, exprs: &[Expr; 3], role: &str) -> Result<Vec3, String> {
    let mut v = [0.0; 3];
    for (slot, e) in v.iter_mut().zip(exprs) {
        let q = variables
            .evaluate(e.source())
            .map_err(|err| format!("{role}: `{}`: {err}", e.source()))?;
        *slot = q.value;
    }
    Vec3::new(v[0], v[1], v[2])
        .normalized()
        .filter(|d| d.is_finite())
        .ok_or_else(|| {
            format!(
                "{role}: the direction ({}, {}, {}) has no length",
                v[0], v[1], v[2]
            )
        })
}

/// The pieces of one rail, each as a spline run the rail's way, with the
/// rail's key and the piece's source.
fn rail_splines(
    doc: &Document,
    earlier: &[FeatureResult],
    rail: &DocRail,
    role: &str,
    tol: f64,
) -> Result<Vec<(NurbsCurve, u64)>, String> {
    let made = rail_of(doc, earlier, rail, role, tol)?;
    let mut out = Vec::with_capacity(made.pieces.len());
    for p in &made.pieces {
        let spline = edge_spline(&p.curve, p.range).map_err(|e| format!("{role}: {e}"))?;
        let spline = if p.reversed {
            reversed(&spline).map_err(|e| format!("{role}: {e}"))?
        } else {
            spline
        };
        out.push((spline, p.source));
    }
    Ok(out)
}

fn ends(c: &NurbsCurve) -> (Vec3, Vec3) {
    let (a, b) = c.domain();
    (c.eval(a), c.eval(b))
}

/// The rails of a loop end to end: each run so it starts where the one
/// before ended (the first so that it ends where the second starts), and
/// the last ending where the first starts, within `tol`.
fn chained(
    rails: Vec<Vec<(NurbsCurve, u64)>>,
    role: &str,
    tol: f64,
) -> Result<Vec<Vec<(NurbsCurve, u64)>>, String> {
    let flip = |pieces: Vec<(NurbsCurve, u64)>| -> Result<Vec<(NurbsCurve, u64)>, String> {
        let mut out = Vec::with_capacity(pieces.len());
        for (c, s) in pieces.into_iter().rev() {
            out.push((reversed(&c).map_err(|e| format!("{role}: {e}"))?, s));
        }
        Ok(out)
    };
    let first_last = |r: &[(NurbsCurve, u64)]| -> Option<(Vec3, Vec3)> {
        let a = ends(&r.first()?.0).0;
        let b = ends(&r.last()?.0).1;
        Some((a, b))
    };
    let count = rails.len();
    let mut out: Vec<Vec<(NurbsCurve, u64)>> = Vec::with_capacity(count);
    for (i, rail) in rails.into_iter().enumerate() {
        let Some((a, b)) = first_last(&rail) else {
            return Err(format!("{role}: curve {} has no pieces", i + 1));
        };
        let Some(previous) = out.last() else {
            out.push(rail);
            continue;
        };
        let Some((pa, pb)) = first_last(previous) else {
            return Err(format!("{role}: a curve lost its pieces; this is a bug"));
        };
        if pb.distance(a) <= tol {
            out.push(rail);
        } else if pb.distance(b) <= tol {
            out.push(flip(rail)?);
        } else if i == 1 && (pa.distance(a) <= tol || pa.distance(b) <= tol) {
            // The first one runs the other way.
            let first = out
                .pop()
                .ok_or_else(|| format!("{role}: a curve lost its pieces"))?;
            out.push(flip(first)?);
            out.push(if pa.distance(a) <= tol {
                rail
            } else {
                flip(rail)?
            });
        } else {
            return Err(format!(
                "{role}: curve {} does not start or end where curve {i} ends (gap {:.3e}; the tolerance is {tol:.3e})",
                i + 1,
                pb.distance(a).min(pb.distance(b))
            ));
        }
    }
    let (Some(first), Some(last)) = (out.first(), out.last()) else {
        return Err(format!("{role}: a loop needs curves"));
    };
    let (Some((start, _)), Some((_, end))) = (first_last(first), first_last(last)) else {
        return Err(format!("{role}: a loop needs curves"));
    };
    if start.distance(end) > tol {
        return Err(format!(
            "{role}: the curves do not close: the loop ends {:.3e} from where it starts (the tolerance is {tol:.3e})",
            start.distance(end)
        ));
    }
    Ok(out)
}

/// A spline surface's parameter domain as a frame for parameter-space
/// sketches: `(x, y)` in percent, `[0, 100]²`, to `(u, v)` (percent, not
/// `[0, 1]`, so the sketch's curves stay longer than the document's
/// tolerance at every scale it is used at).
fn to_domain(surface: &NurbsSurface, x: f64, y: f64) -> (f64, f64) {
    let ((u0, u1), (v0, v1)) = surface.domain();
    (u0 + x / 100.0 * (u1 - u0), v0 + y / 100.0 * (v1 - v0))
}

/// The pieces of a sketch's rail in the sketch's own coordinates, as
/// parameter-space curves on `surface` (exact: an affine map of the
/// control points).
fn parameter_pieces(
    doc: &Document,
    earlier: &[FeatureResult],
    rail: &DocRail,
    surface: &NurbsSurface,
    role: &str,
    tol: f64,
) -> Result<Vec<(NurbsCurve, u64)>, String> {
    let DocRail::Sketch(sketch) = rail else {
        return Err(format!(
            "{role}: in parameter space a curve is a sketch (its x and y read as u and v in percent of the domain), not edges"
        ));
    };
    let plane: Frame =
        crate::extrude::drawn_plain(doc, earlier, *sketch, "a curve in parameter space")
            .map_err(|e| format!("{role}: {e}"))?
            .plane;
    let mut out = Vec::new();
    for (spline, source) in rail_splines(doc, earlier, rail, role, tol)? {
        let ctrl: Vec<Vec3> = spline
            .control_points()
            .iter()
            .map(|p| {
                let (x, y, _) = plane.local(*p);
                let (u, v) = to_domain(surface, x, y);
                Vec3::new(u, v, 0.0)
            })
            .collect();
        let uv = NurbsCurve::new(
            spline.degree(),
            spline.knots().to_vec(),
            ctrl,
            spline.weights().to_vec(),
        )
        .map_err(|e| format!("{role}: {e}"))?;
        out.push((uv, source));
    }
    Ok(out)
}

/// A curve projected onto a plane: an affine map of its control points,
/// exact.
fn onto_plane(
    curve: &NurbsCurve,
    plane: &Frame,
    along: Option<Vec3>,
    role: &str,
) -> Result<NurbsCurve, String> {
    let n = plane.z();
    let o = plane.origin();
    let d = match along {
        Some(d) => {
            if d.dot(n).abs() < 0.05 {
                return Err(format!(
                    "{role}: the direction runs along the plane (within 3°), so its lines do not meet it"
                ));
            }
            d
        }
        None => n,
    };
    let ctrl: Vec<Vec3> = curve
        .control_points()
        .iter()
        .map(|p| *p - d * ((*p - o).dot(n) / d.dot(n)))
        .collect();
    NurbsCurve::new(
        curve.degree(),
        curve.knots().to_vec(),
        ctrl,
        curve.weights().to_vec(),
    )
    .map_err(|e| format!("{role}: {e}"))
}

/// Twice the signed area a loop of parameter-space points encloses.
fn signed_area(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points.get(i), points.get((i + 1) % n));
            match (a, b) {
                (Some(a), Some(b)) => a.0 * b.1 - b.0 * a.1,
                _ => 0.0,
            }
        })
        .sum()
}

/// The loops of a trimmed face, each its pieces in order, with the
/// largest projection gap found.
pub(crate) fn build_trim(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    feature: &SurfaceTrim,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Trimmed), String> {
    let tol = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let (source_body, names, made_by) = named_body_of(doc, earlier, &feature.face, None, "face")?;
    let index = (0..source_body.faces.len())
        .find(|&i| names.face_answers_to(i, &feature.face))
        .ok_or_else(|| format!("face: `{made_by}` has no face {}", feature.face))?;
    let source = source_body
        .faces
        .get(index)
        .ok_or_else(|| format!("face: `{made_by}` has no face {}", feature.face))?;
    let surface = source.surface.clone();
    let face_reversed = source.reversed;
    let analytic = matches!(
        surface,
        Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_) | Surface::Torus(_)
    );
    if feature.space == TrimSpace::Parameter && (analytic || matches!(surface, Surface::Plane(_))) {
        return Err(format!(
            "space: a {} has no parameter square here; give its curves in model space",
            surface.kind()
        ));
    }
    let along = match &feature.along {
        Some(exprs) => Some(direction(variables, exprs, "along")?),
        None => None,
    };
    if along.is_some() && feature.space == TrimSpace::Parameter {
        return Err("along: curves in parameter space are not projected".into());
    }
    let how = along.map_or(Projection::Normal, Projection::Along);
    // The spline the curves are made exact on: the face's own, or for an
    // analytic face its rational form about where the curves land (the
    // same surface: the face keeps its analytic kind).
    let spline: Option<NurbsSurface> = match &surface {
        Surface::Nurbs(n) => Some(n.clone()),
        Surface::Bezier(b) => Some(NurbsSurface::from_bezier(b).map_err(|e| format!("face: {e}"))?),
        Surface::Plane(_) => None,
        _ => {
            let mut landed = Vec::new();
            for (l, rails) in std::iter::once(&feature.outer)
                .chain(feature.holes.iter())
                .enumerate()
            {
                for (i, rail) in rails.iter().enumerate() {
                    let role = format!(
                        "{}[{i}]",
                        if l == 0 {
                            "outer".to_string()
                        } else {
                            format!("holes[{}]", l - 1)
                        }
                    );
                    for (curve, _) in rail_splines(doc, earlier, rail, &role, tol.linear)? {
                        let (a, b) = curve.domain();
                        for k in 0..=8 {
                            let p = curve.eval(a + (b - a) * f64::from(k) / 8.0);
                            let (u, v) = trim::project_point(&surface, p, how, None, tol.linear)
                                .map_err(|e| format!("{role}: {e}"))?;
                            landed.push(surface.eval(u, v));
                        }
                    }
                }
            }
            Some(trim::rational_form(&surface, &landed).map_err(|e| format!("face: {e}"))?)
        }
    };
    // Whether the spline's parameters turn the other way about the face's
    // normal than the face's own (an analytic face's rational form).
    // Compared by the face's closed-form normal (which holds at a sphere's
    // pole, where its parameter derivatives cross to nothing), at the
    // first of a few places of the form where both normals are defined
    // (a cone's apex has none).
    let turned = match (&spline, analytic) {
        (Some(s), true) => {
            let ((u0, u1), (v0, v1)) = s.domain();
            let mut verdict = None;
            'places: for (x, y) in [
                (0.5, 0.5),
                (0.25, 0.25),
                (0.75, 0.25),
                (0.25, 0.75),
                (0.75, 0.75),
            ] {
                let (um, vm) = (u0 + x * (u1 - u0), v0 + y * (v1 - v0));
                let (p, a, b) = s.derivatives(um, vm);
                let Some(n_form) = a.cross(b).normalized() else {
                    continue 'places;
                };
                let (fu, fv) = surface.project(p).map_err(|e| format!("face: {e}"))?;
                let Some(n_face) = surface.normal(fu, fv) else {
                    continue 'places;
                };
                let d = n_form.dot(n_face);
                if d.abs() > 0.5 {
                    verdict = Some(d < 0.0);
                    break;
                }
            }
            verdict.ok_or_else(|| {
                format!(
                    "face: the {}'s rational form and the face have no normal to compare where the curves land (an apex or pole under the whole region)",
                    surface.kind()
                )
            })?
        }
        _ => false,
    };
    let mut deviation = 0.0_f64;
    let mut degree = 0usize;
    // Each loop: its pieces, and its points in the surface's parameters
    // (for its turning).
    let mut loops: Vec<TrimLoop> = Vec::new();
    let all_loops = std::iter::once(&feature.outer).chain(feature.holes.iter());
    for (l, rails) in all_loops.enumerate() {
        let role = if l == 0 {
            "outer".to_string()
        } else {
            format!("holes[{}]", l - 1)
        };
        if rails.is_empty() {
            return Err(format!("{role}: a loop needs at least one curve"));
        }
        // The rails' pieces: in space for model-space curves, in
        // parameters for parameter-space ones.
        let mut per_rail: Vec<Vec<(NurbsCurve, u64)>> = Vec::with_capacity(rails.len());
        let mut keys: Vec<u64> = Vec::with_capacity(rails.len());
        for (i, rail) in rails.iter().enumerate() {
            let role = format!("{role}[{i}]");
            keys.push(key(rail));
            per_rail.push(match (&feature.space, &spline) {
                (TrimSpace::Parameter, Some(s)) => {
                    parameter_pieces(doc, earlier, rail, s, &role, tol.linear)?
                }
                _ => rail_splines(doc, earlier, rail, &role, tol.linear)?,
            });
        }
        // Chained in the space they are in (parameter-space curves are
        // closed within a rounding of the square).
        let chain_tol = match feature.space {
            TrimSpace::Parameter => 1e-9,
            TrimSpace::Model => tol.linear,
        };
        // Keep the keys with their rails while chaining.
        let tagged: Vec<Vec<(NurbsCurve, u64)>> = per_rail;
        let chained_rails = chained(tagged, &role, chain_tol)?;
        let mut pieces: Vec<Piece> = Vec::new();
        let mut uvs: Vec<(f64, f64)> = Vec::new();
        for (rail_index, rail) in chained_rails.iter().enumerate() {
            let k = keys.get(rail_index).copied().unwrap_or(0);
            for (curve, source) in rail {
                let what = format!("{role}[{rail_index}]");
                let (space, uv) = match (&spline, &surface) {
                    (Some(s), _) => {
                        let uv = match feature.space {
                            TrimSpace::Parameter => curve.clone(),
                            TrimSpace::Model => {
                                let (uv, gap) = trim::project_points(
                                    &Surface::Nurbs(s.clone()),
                                    &Curve3::Nurbs(curve.clone()),
                                    curve.domain(),
                                    how,
                                    0.5 * tol.linear,
                                    tol.linear,
                                )
                                .map_err(|e| format!("{what}: {e}"))?;
                                deviation = deviation.max(gap);
                                uv
                            }
                        };
                        let space = trim::compose(s, &uv).map_err(|e| format!("{what}: {e}"))?;
                        (space, Some(uv))
                    }
                    (None, Surface::Plane(plane)) => {
                        (onto_plane(curve, plane.frame(), along, &what)?, None)
                    }
                    _ => return Err(format!("{what}: no surface to put it on; this is a bug")),
                };
                degree = degree.max(space.degree());
                // Points for the turning: 16 per piece.
                let (a, b) = space.domain();
                for i in 0..16 {
                    let t = a + (b - a) * i as f64 / 16.0;
                    uvs.push(match (&uv, &surface) {
                        (Some(uv), _) => {
                            let q = uv.eval(t);
                            (q.x, q.y)
                        }
                        (None, Surface::Plane(plane)) => {
                            let (x, y, _) = plane.frame().local(space.eval(t));
                            (x, y)
                        }
                        _ => (0.0, 0.0),
                    });
                }
                pieces.push(Piece {
                    curve: space,
                    key: k,
                    source: *source,
                });
            }
        }
        loops.push((pieces, uvs));
    }
    // Turn each loop: the outer one counter-clockwise about the outward
    // normal in the parameters, the holes the other way.
    let mut sizes = Vec::with_capacity(loops.len());
    let mut body = Body::new(BodyKind::Sheet);
    let mut edge_names: Vec<PersistentName> = Vec::new();
    let mut vertex_names: Vec<PersistentName> = Vec::new();
    let mut loop_ids = Vec::with_capacity(loops.len());
    for (l, (pieces, uvs)) in loops.into_iter().enumerate() {
        let area = signed_area(&uvs);
        if area == 0.0 || !area.is_finite() {
            return Err(format!(
                "{}: the loop encloses nothing on the surface",
                if l == 0 {
                    "outer".to_string()
                } else {
                    format!("holes[{}]", l - 1)
                }
            ));
        }
        let want_positive = ((l == 0) != face_reversed) != turned;
        let pieces: Vec<Piece> = if (area > 0.0) == want_positive {
            pieces
        } else {
            let mut out = Vec::with_capacity(pieces.len());
            for p in pieces.into_iter().rev() {
                out.push(Piece {
                    curve: reversed(&p.curve).map_err(|e| e.to_string())?,
                    key: p.key,
                    source: p.source,
                });
            }
            out
        };
        sizes.push(pieces.len());
        // A vertex at the start of every piece.
        let count = pieces.len();
        let mut vertices: Vec<VertexId> = Vec::with_capacity(count);
        for (i, p) in pieces.iter().enumerate() {
            let (start, _) = ends(&p.curve);
            // The vertex between the piece before and this one.
            let before = pieces
                .get((i + count - 1) % count)
                .ok_or("a loop lost its pieces; this is a bug")?;
            let (_, previous_end) = ends(&before.curve);
            let at = start.lerp(previous_end, 0.5);
            vertices.push(body.add_vertex(at));
            // The two (key, source) pairs in a fixed order, so the name
            // does not depend on which way the loop runs.
            let (first, second) = {
                let (x, y) = ((before.key, before.source), (p.key, p.source));
                if x <= y { (x, y) } else { (y, x) }
            };
            vertex_names.push(PersistentName::new(
                id,
                NameRole::StartVertex,
                vec![first.0, first.1, second.0, second.1],
            ));
        }
        let mut coedges = Vec::with_capacity(count);
        for (i, p) in pieces.iter().enumerate() {
            let (Some(&start), Some(&end)) = (vertices.get(i), vertices.get((i + 1) % count))
            else {
                return Err("a loop lost its vertices; this is a bug".into());
            };
            let edge = body.add_edge(Curve3::Nurbs(p.curve.clone()), p.curve.domain(), start, end);
            coedges.push(Coedge {
                edge,
                reversed: false,
            });
            edge_names.push(PersistentName::new(
                id,
                NameRole::StartEdge,
                vec![p.key, p.source],
            ));
        }
        loop_ids.push(body.add_loop(coedges));
    }
    let face = body.add_face(surface, face_reversed, loop_ids);
    body.add_shell(vec![face]);
    // Two entities with one name would be taken as pieces of one.
    for list in [&edge_names, &vertex_names] {
        let distinct: std::collections::BTreeSet<String> =
            list.iter().map(ToString::to_string).collect();
        if distinct.len() != list.len() {
            return Err(
                "two edges or vertices of the face would have one name (a curve used twice in the loops)".into(),
            );
        }
    }
    let names = Names::listed(
        vec![PersistentName::new(id, NameRole::Face, vec![0])],
        edge_names,
        vertex_names,
    );
    Ok((
        body,
        names,
        Trimmed {
            deviation,
            degree,
            loops: sizes,
        },
    ))
}

/// What an N-sided fill made beside its body: how its inner seams and
/// its held curves meet, proven.
#[derive(Clone, Debug, PartialEq)]
pub struct Filled {
    /// Per inner seam (spoke `k`, to curve `k`'s midpoint): what was asked
    /// and built (`true` for curvature), why less where less, and the
    /// proof.
    pub seams: Vec<FilledSeam>,
    /// Per curve held tangent or in curvature: what was asked and built
    /// (`true` for curvature), why less, and the proofs along its two
    /// halves.
    pub outer: Vec<Option<FilledEdge>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilledEdge {
    pub asked_curvature: bool,
    pub built_curvature: bool,
    pub why: Option<String>,
    pub halves: Vec<craft3d_geom::network::Continuity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilledSeam {
    pub asked_curvature: bool,
    pub built_curvature: bool,
    pub why: Option<String>,
    pub continuity: craft3d_geom::network::Continuity,
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

pub(crate) fn build_nsided(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    feature: &SurfaceNsided,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Filled), String> {
    use craft3d_geom::network::nsided::{self, Inner, Nsided};
    use craft3d_geom::network::{self, Condition};
    let tol = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let n = feature.curves.len();
    if !(nsided::MIN_SIDES..=nsided::MAX_SIDES).contains(&n) {
        return Err(format!(
            "curves: an N-sided fill has {} to {} curves, got {n}",
            nsided::MIN_SIDES,
            nsided::MAX_SIDES
        ));
    }
    if !feature.edges.is_empty() && feature.edges.len() != n {
        return Err(format!(
            "edges: one hold per curve ({n}), got {}",
            feature.edges.len()
        ));
    }
    // Each curve one spline, run so the loop goes round.
    let mut per_rail = Vec::with_capacity(n);
    let mut keys = Vec::with_capacity(n);
    for (i, rail) in feature.curves.iter().enumerate() {
        let role = format!("curves[{i}]");
        keys.push(key(rail));
        per_rail.push(rail_splines(doc, earlier, rail, &role, tol.linear)?);
    }
    let chained_rails = chained(per_rail, "curves", tol.linear)?;
    let mut curves = Vec::with_capacity(n);
    for (i, pieces) in chained_rails.iter().enumerate() {
        let splines: Vec<NurbsCurve> = pieces.iter().map(|(c, _)| c.clone()).collect();
        let joined = network::chain(&splines, tol.linear, tol.angular)
            .map_err(|e| format!("curves[{i}]: {e}"))?;
        curves.push(joined);
    }
    let distinct: std::collections::BTreeSet<u64> = keys.iter().copied().collect();
    if distinct.len() != n {
        return Err(
            "curves: two curves are known by one name (the same sketch, or edges named alike)"
                .into(),
        );
    }
    // The holds' faces, then the conditions that borrow them.
    let mut faces: Vec<Option<(Surface, bool)>> = Vec::with_capacity(n);
    for k in 0..n {
        let hold = feature.edges.get(k).cloned().unwrap_or(EdgeHold::Free);
        let role = format!("edges[{k}]");
        faces.push(match &hold {
            EdgeHold::Free => None,
            EdgeHold::Position { face } => Some((face_surface(doc, earlier, face, &role)?, false)),
            EdgeHold::Tangent { face, .. } | EdgeHold::Curvature { face, .. } => {
                Some((face_surface(doc, earlier, face, &role)?, true))
            }
        });
    }
    let conditions: Vec<Condition<'_>> = faces
        .iter()
        .zip(0..n)
        .map(|(f, k)| match (f, feature.edges.get(k)) {
            (Some((s, false)), _) => Condition::Position(s),
            (Some((s, true)), Some(EdgeHold::Curvature { .. })) => Condition::Curvature {
                surface: s,
                weight: 1.0,
            },
            (Some((s, true)), _) => Condition::Tangent {
                surface: s,
                weight: 1.0,
            },
            _ => Condition::Free,
        })
        .collect();
    // A position hold: the curve lies on its face (checked).
    for (k, (curve, cond)) in curves.iter().zip(&conditions).enumerate() {
        if let Condition::Position(face) = cond {
            let (a, b) = curve.domain();
            for i in 0..=64 {
                let p = curve.eval(a + (b - a) * f64::from(i) / 64.0);
                let (u, v) = face.project(p).map_err(|e| format!("edges[{k}]: {e}"))?;
                let d = face.eval(u, v).distance(p);
                if d > tol.linear {
                    return Err(format!(
                        "edges[{k}]: curve {} is {d:.3e} off its face (the tolerance is {:.3e})",
                        k + 1,
                        tol.linear
                    ));
                }
            }
        }
    }
    let inner = match feature.inner {
        InnerSeams::Tangent => Inner::Tangent,
        InnerSeams::Curvature => Inner::Curvature,
    };
    let rise = match &feature.rise {
        Some(e) => {
            let q = variables
                .evaluate(e.source())
                .map_err(|err| format!("rise: `{}`: {err}", e.source()))?;
            if !q.value.is_finite() {
                return Err(format!("rise: `{}` is not a number", e.source()));
            }
            q.value
        }
        None => 0.0,
    };
    let made = nsided::nsided(&Nsided {
        curves: &curves,
        edges: &conditions,
        inner,
        rise,
        tolerance: tol.linear,
        angular: tol.angular,
    })
    .map_err(|e| format!("curves: {e}"))?;
    // The body: the centre, the midpoints and the corners; the spokes and
    // the halves; one face per patch.
    let mut body = Body::new(BodyKind::Sheet);
    let mut vertex_names = Vec::with_capacity(2 * n + 1);
    let centre = body.add_vertex(made.centre);
    vertex_names.push(PersistentName::new(id, NameRole::StartVertex, vec![0]));
    let mut corners = Vec::with_capacity(n);
    let mut mids = Vec::with_capacity(n);
    for k in 0..n {
        let (first, _) = made
            .halves
            .get(k)
            .ok_or("a fill lost its curves; this is a bug")?;
        let (a, b) = first.domain();
        let kk = keys.get(k).copied().unwrap_or(0);
        let before = keys.get((k + n - 1) % n).copied().unwrap_or(0);
        corners.push(body.add_vertex(first.eval(a)));
        vertex_names.push(PersistentName::new(
            id,
            NameRole::EndVertex,
            vec![before, kk],
        ));
        mids.push(body.add_vertex(first.eval(b)));
        vertex_names.push(PersistentName::new(id, NameRole::StartVertex, vec![kk]));
    }
    let mut edge_names = Vec::with_capacity(3 * n);
    let mut spoke_edges = Vec::with_capacity(n);
    let mut half_edges = Vec::with_capacity(n);
    for k in 0..n {
        let kk = keys.get(k).copied().unwrap_or(0);
        let spoke = made
            .spokes
            .get(k)
            .ok_or("a fill lost its spokes; this is a bug")?;
        let (Some(&m), Some(&v), Some(&v_next)) =
            (mids.get(k), corners.get(k), corners.get((k + 1) % n))
        else {
            return Err("a fill lost its vertices; this is a bug".into());
        };
        spoke_edges.push(body.add_edge(Curve3::Nurbs(spoke.clone()), spoke.domain(), centre, m));
        edge_names.push(PersistentName::new(id, NameRole::LateralEdge, vec![kk]));
        let (first, second) = made
            .halves
            .get(k)
            .ok_or("a fill lost its curves; this is a bug")?;
        let e1 = body.add_edge(Curve3::Nurbs(first.clone()), first.domain(), v, m);
        edge_names.push(PersistentName::new(id, NameRole::StartEdge, vec![kk]));
        let e2 = body.add_edge(Curve3::Nurbs(second.clone()), second.domain(), m, v_next);
        edge_names.push(PersistentName::new(id, NameRole::EndEdge, vec![kk]));
        half_edges.push((e1, e2));
    }
    let mut face_ids = Vec::with_capacity(n);
    let mut face_names = Vec::with_capacity(n);
    for k in 0..n {
        let next = (k + 1) % n;
        let (Some(&sk), Some(&sn), Some(&(_, ek_second)), Some(&(en_first, _))) = (
            spoke_edges.get(k),
            spoke_edges.get(next),
            half_edges.get(k),
            half_edges.get(next),
        ) else {
            return Err("a fill lost its edges; this is a bug".into());
        };
        let lp = body.add_loop(vec![
            Coedge {
                edge: sk,
                reversed: false,
            },
            Coedge {
                edge: ek_second,
                reversed: false,
            },
            Coedge {
                edge: en_first,
                reversed: false,
            },
            Coedge {
                edge: sn,
                reversed: true,
            },
        ]);
        let surface = made
            .patches
            .get(k)
            .ok_or("a fill lost its patches; this is a bug")?;
        face_ids.push(body.add_face(Surface::Nurbs(surface.clone()), false, vec![lp]));
        face_names.push(PersistentName::new(
            id,
            NameRole::Side,
            vec![
                keys.get(k).copied().unwrap_or(0),
                keys.get(next).copied().unwrap_or(0),
            ],
        ));
    }
    body.add_shell(face_ids);
    let names = Names::listed(face_names, edge_names, vertex_names);
    let report = Filled {
        seams: made
            .seams
            .into_iter()
            .map(|s| FilledSeam {
                asked_curvature: s.asked == Inner::Curvature,
                built_curvature: s.built == Inner::Curvature,
                why: s.why,
                continuity: s.continuity,
            })
            .collect(),
        outer: made
            .outer
            .into_iter()
            .map(|o| {
                o.map(|h| FilledEdge {
                    asked_curvature: h.asked == Inner::Curvature,
                    built_curvature: h.built == Inner::Curvature,
                    why: h.why,
                    halves: h.halves,
                })
            })
            .collect(),
    };
    Ok((body, names, report))
}
