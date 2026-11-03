//! Whether a body is a plate: a prism of constant thickness, its top and
//! bottom two parallel flat faces facing away from each other, every
//! other face square to them (side walls, holes through), so that it is
//! cut from sheet stock along its outline.
//!
//! The test, per axis along which the body has a flat face on both sides
//! at its extremes (its extent along the axis is the thickness): the top
//! and the bottom are one flat face each; every other face is square to
//! the axis (a plane whose normal is square to it, a cylinder along it, a
//! freeform face whose normal is square to it along its edges); every
//! vertex lies in the top's plane or the bottom's; the top and the bottom
//! have the same number of loops and the same area. Of the axes that pass,
//! the one with the least thickness is the plate's (a box is a plate of
//! its smallest side); when a thinner axis failed (a plate with one long
//! edge chamfered is a prism along that edge), the plate says why. When
//! none passes, the reason is the thinnest axis's, named by face.

use super::Shape;
use crate::drawing::{FaceAxes, face_outline_in};
use craft3d_geom::Surface;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId};

/// Most axes looked at; a body with more distinct directions of flat
/// faces on both sides is refused.
const MAX_AXES: usize = 4096;
/// Most axes tested in full (the thinnest first).
const MAX_TESTED: usize = 1024;

/// A plate: its thickness, its top and bottom, and its outline and holes
/// drawn in the top's drawing frame (`frame`: seen from outside, y up the
/// face or, lying flat, x along the model's x; as `file.export` draws a
/// face).
#[derive(Clone, Debug)]
pub struct Plate {
    pub thickness: f64,
    pub top: FaceId,
    pub bottom: FaceId,
    pub frame: Frame,
    pub shape: Shape,
    /// When the body is a prism along this axis but not along a thinner
    /// one: why not (a bar whose long edge is chamfered is a prism along
    /// its length, its outline the chamfered section), with that thickness.
    pub thinner: Option<(f64, NotPlate)>,
}

/// Why a body is not a plate, and the faces that say so.
#[derive(Clone, Debug, PartialEq)]
pub struct NotPlate {
    pub reason: String,
    pub faces: Vec<FaceId>,
}

fn not(reason: impl Into<String>, faces: Vec<FaceId>) -> NotPlate {
    NotPlate {
        reason: reason.into(),
        faces,
    }
}

/// A flat face: its outward normal and its plane's level along it.
struct Flat {
    face: FaceId,
    normal: Vec3,
    level: f64,
}

fn flats(body: &Body) -> Vec<Flat> {
    let mut out = Vec::new();
    for (i, face) in body.faces.iter().enumerate() {
        let Surface::Plane(plane) = &face.surface else {
            continue;
        };
        let z = plane.frame().z();
        let normal = if face.reversed { -z } else { z };
        let Some(normal) = normal.normalized() else {
            continue;
        };
        let Ok(id) = u32::try_from(i) else { break };
        out.push(Flat {
            face: FaceId(id),
            normal,
            level: normal.dot(plane.frame().origin()),
        });
    }
    out
}

/// The vertices of a face (its edges' ends).
fn face_vertices(body: &Body, face: FaceId) -> Vec<Vec3> {
    let mut out = Vec::new();
    let Some(f) = body.face(face) else {
        return out;
    };
    for l in &f.loops {
        let Some(lp) = body.loop_(*l) else { continue };
        for c in &lp.coedges {
            let Some(e) = body.edge(c.edge) else { continue };
            for v in [e.start, e.end] {
                if let Some(v) = body.vertex(v) {
                    out.push(v.point);
                }
            }
        }
    }
    out
}

/// Points along a face's edges: their ends and three between.
fn edge_points(body: &Body, face: FaceId) -> Vec<Vec3> {
    let mut out = Vec::new();
    let Some(f) = body.face(face) else {
        return out;
    };
    for l in &f.loops {
        let Some(lp) = body.loop_(*l) else { continue };
        for c in &lp.coedges {
            let Some(e) = body.edge(c.edge) else { continue };
            let (a, b) = e.range;
            for i in 0..5 {
                out.push(e.curve.eval(a + (b - a) * f64::from(i) / 4.0));
            }
        }
    }
    out
}

/// The angle in degrees between a face whose normal is `n` and the plate
/// (whose normal is `axis`): 90 for a side wall.
fn degrees_to_plate(n: Vec3, axis: Vec3) -> f64 {
    let c = n.dot(axis).abs().min(1.0);
    90.0 - c.pacos().to_degrees()
}

/// Why the surface of face `face` keeps the body from being a plate along
/// `axis` whose top is at `hi` (it is not square to the plate), or `None`
/// when it does not.
fn side_fault(
    body: &Body,
    face: FaceId,
    axis: Vec3,
    (lo, hi): (f64, f64),
    tolerance: &Tolerance,
    name: &dyn Fn(FaceId) -> String,
) -> Option<String> {
    let f = body.face(face)?;
    let square = tolerance.angular.max(1e-9);
    let what = name(face);
    match &f.surface {
        Surface::Plane(plane) => {
            let n = plane.frame().z();
            if n.cross(axis).length() <= square {
                let depth = hi - axis.dot(plane.frame().origin());
                return Some(format!(
                    "face {what} is flat and parallel to the plate, {depth} below its top: a pocket's floor, a step or a counterbore"
                ));
            }
            if n.dot(axis).abs() > square {
                return Some(format!(
                    "face {what} is flat at {:.6} degrees to the plate, not square to it: a chamfer, a bevel or a draft",
                    degrees_to_plate(n, axis)
                ));
            }
        }
        Surface::Cylinder(cylinder) => {
            if cylinder.frame().z().cross(axis).length() > square {
                return Some(format!(
                    "face {what} is a cylinder whose axis is not square to the plate: a round on an edge, or a hole at a slant"
                ));
            }
        }
        Surface::Cone(_) => {
            return Some(format!(
                "face {what} is a cone: a countersink or a chamfered hole, not square to the plate"
            ));
        }
        Surface::Sphere(_) | Surface::Torus(_) => {
            return Some(format!(
                "face {what} is a {}: a round, not square to the plate",
                f.surface.kind()
            ));
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            // Accepted only on a certificate: the control net is an
            // extrusion along the axis. Otherwise refused; samples only
            // say how it fails.
            if !extruded_along(&f.surface, axis, tolerance) {
                return Some(not_a_wall(
                    body,
                    face,
                    &f.surface,
                    axis,
                    (lo, hi),
                    tolerance,
                    &what,
                ));
            }
        }
    }
    None
}

/// A control point of a net and its weight, by its indices along u and v.
type NetPoint<'a> = Box<dyn Fn(usize, usize) -> Option<(Vec3, f64)> + 'a>;

/// Whether the freeform `surface` lies on the cylinder over a curve along
/// `axis`: in one parameter direction every row of control points
/// projects to one point of the plate's plane (within ten tolerances) and
/// the weights factor (`w_ij = a_i b_j`, within 1e-12). Then every point
/// of the surface, `Σ N_i N_j w_ij P_ij / Σ N_i N_j w_ij`, projects to the
/// rational curve of the projected rows with the weights `a_i`, whatever
/// the other parameter: the wall is square to the plate. A certificate,
/// not a sample.
fn extruded_along(surface: &Surface, axis: Vec3, tolerance: &Tolerance) -> bool {
    let (nu, nv, point): (usize, usize, NetPoint<'_>) = match surface {
        Surface::Nurbs(n) => {
            let (nu, nv) = n.counts();
            let (ctrl, weights) = (n.control_points(), n.weights());
            (
                nu,
                nv,
                Box::new(move |i, j| {
                    let k = i.checked_mul(nv)?.checked_add(j)?;
                    Some((*ctrl.get(k)?, *weights.get(k)?))
                }),
            )
        }
        Surface::Bezier(b) => (
            b.degree_u() + 1,
            b.degree_v() + 1,
            Box::new(move |i, j| Some((b.control_point(i, j)?, 1.0))),
        ),
        _ => return false,
    };
    if nu == 0 || nv == 0 {
        return false;
    }
    let flat = |p: Vec3| p - axis * axis.dot(p);
    let on = 10.0 * tolerance.linear;
    let factor = |i: usize, j: usize| -> Option<bool> {
        let (_, w) = point(i, j)?;
        let (_, w00) = point(0, 0)?;
        let (_, wi0) = point(i, 0)?;
        let (_, w0j) = point(0, j)?;
        let (x, y) = (w * w00, wi0 * w0j);
        Some((x - y).abs() <= 1e-12 * x.abs().max(y.abs()))
    };
    // Rows along v (i fixed) or along u (j fixed) to one point each.
    let rows = |along_v: bool| -> bool {
        let (outer, inner) = if along_v { (nu, nv) } else { (nv, nu) };
        (0..outer).all(|a| {
            let at = |b: usize| if along_v { (a, b) } else { (b, a) };
            let Some((first, _)) = point(at(0).0, at(0).1) else {
                return false;
            };
            (0..inner).all(|b| {
                let (i, j) = at(b);
                point(i, j).is_some_and(|(p, _)| (flat(p) - flat(first)).length() <= on)
                    && factor(i, j) == Some(true)
            })
        })
    };
    rows(true) || rows(false)
}

/// Why the freeform `surface` of `face`, whose control net is no
/// extrusion along `axis`, is no wall square to the plate: where points of
/// its edges carried along the axis leave it (each measured against the
/// edge point's own distance from the surface, so an edge held to a
/// looser tolerance is not counted against it), or where its normal is
/// not square to the axis; else that it is not proven.
fn not_a_wall(
    body: &Body,
    face: FaceId,
    surface: &Surface,
    axis: Vec3,
    (lo, hi): (f64, f64),
    tolerance: &Tolerance,
    what: &str,
) -> String {
    let on = 10.0 * tolerance.linear;
    let distance = |q: Vec3| {
        surface
            .project(q)
            .ok()
            .map(|(u, v)| ((surface.eval(u, v) - q).length(), surface.normal(u, v)))
    };
    for p in edge_points(body, face) {
        let Some((own, _)) = distance(p) else {
            continue;
        };
        let level = axis.dot(p);
        for j in 0..=8 {
            let q = p + axis * (lo + (hi - lo) * f64::from(j) / 8.0 - level);
            match distance(q) {
                Some((d, _)) if d - own > on => {
                    return format!(
                        "face {what} is a freeform face that is no straight wall: it leaves the line square to the plate through its edge by {}",
                        d - own
                    );
                }
                Some((_, Some(n))) if n.dot(axis).abs() > tolerance.angular * n.length() => {
                    return format!("face {what} is a freeform face not square to the plate");
                }
                _ => {}
            }
        }
    }
    format!(
        "face {what} is a freeform face not proven square to the plate: its control net is no extrusion along the plate's normal"
    )
}

/// Why face `face` has a corner between the plate's top at `hi` and its
/// bottom at `lo`, or `None` when it has none.
fn level_fault(
    body: &Body,
    face: FaceId,
    axis: Vec3,
    (lo, hi): (f64, f64),
    near: f64,
    name: &dyn Fn(FaceId) -> String,
) -> Option<String> {
    for p in face_vertices(body, face) {
        let level = axis.dot(p);
        if (level - lo).abs() > near && (level - hi).abs() > near {
            return Some(format!(
                "face {} has a corner {} below the plate's top, between its top and bottom: a step, a pocket or a chamfer",
                name(face),
                hi - level
            ));
        }
    }
    None
}

/// Whether the body is a plate along `axis`, with the top facing `+axis`
/// at `hi` and the bottom facing `-axis` at `lo`.
fn plate_along(
    body: &Body,
    flats: &[Flat],
    axis: Vec3,
    (lo, hi): (f64, f64),
    tolerance: Tolerance,
    near: f64,
    name: &dyn Fn(FaceId) -> String,
) -> Result<Plate, NotPlate> {
    let parallel = tolerance.angular.max(1e-9);
    let at = |f: &&Flat, side: f64, level: f64| {
        (f.normal * side).cross(axis).length() <= parallel
            && f.normal.dot(axis) * side > 0.0
            && (f.level * side - level).abs() <= near
    };
    let tops: Vec<FaceId> = flats
        .iter()
        .filter(|f| at(f, 1.0, hi))
        .map(|f| f.face)
        .collect();
    let bottoms: Vec<FaceId> = flats
        .iter()
        .filter(|f| at(f, -1.0, lo))
        .map(|f| f.face)
        .collect();
    let list = |faces: &[FaceId]| {
        faces
            .iter()
            .map(|f| name(*f))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (top, bottom) = match (tops.as_slice(), bottoms.as_slice()) {
        ([top], [bottom]) => (*top, *bottom),
        ([_], several) | (several, [_]) if several.len() > 1 => {
            return Err(not(
                format!(
                    "the plate's side is {} flat faces in one plane ({}), not one",
                    several.len(),
                    list(several)
                ),
                several.to_vec(),
            ));
        }
        _ => {
            let mut faces = tops.clone();
            faces.extend(&bottoms);
            return Err(not(
                format!(
                    "the plate's top or bottom is not one flat face ({})",
                    list(&faces)
                ),
                faces,
            ));
        }
    };
    // Every face's surface first, so that a face at a slant is named
    // rather than its neighbour whose corner it moved; then the corners.
    let sides: Vec<FaceId> = (0..body.faces.len())
        .filter_map(|i| u32::try_from(i).ok().map(FaceId))
        .filter(|f| *f != top && *f != bottom)
        .collect();
    for face in &sides {
        if let Some(reason) = side_fault(body, *face, axis, (lo, hi), &tolerance, name) {
            return Err(not(reason, vec![*face]));
        }
    }
    for face in &sides {
        if let Some(reason) = level_fault(body, *face, axis, (lo, hi), near, name) {
            return Err(not(reason, vec![*face]));
        }
    }
    let outline = |face: FaceId| {
        face_outline_in(body, &[face], FaceAxes::Upright, tolerance)
            .map_err(|e| not(format!("face {}: {e}", name(face)), vec![face]))
    };
    let (seen, under) = (outline(top)?, outline(bottom)?);
    let area = |paths: &[craft3d_drawing::Path]| -> f64 {
        let mut areas: Vec<f64> = paths.iter().map(|p| p.area().abs()).collect();
        areas.sort_by(|a, b| b.total_cmp(a));
        let outer = areas.first().copied().unwrap_or(0.0);
        outer - areas.iter().skip(1).sum::<f64>()
    };
    let (a, b) = (area(&seen.paths), area(&under.paths));
    let length = super::one_layer(seen.paths.clone()).length();
    if seen.paths.len() != under.paths.len() || (a - b).abs() > near * length.max(near) {
        return Err(not(
            format!(
                "the top {} and the bottom {} differ ({} and {} loops, areas {a} and {b}): the body is no prism",
                name(top),
                name(bottom),
                seen.paths.len(),
                under.paths.len()
            ),
            vec![top, bottom],
        ));
    }
    let shape = Shape::from_loops(seen.paths, Vec::new())
        .map_err(|e| not(format!("face {}: {e}", name(top)), vec![top]))?;
    Ok(Plate {
        thickness: hi - lo,
        top,
        bottom,
        frame: seen.frame,
        shape,
        thinner: None,
    })
}

/// The plate the body is, or why it is none. `name` names a face in a
/// reason (its persistent name).
///
/// Which side is the top: the one whose outward normal points most up,
/// by the first of its z, y and x that is not zero; the outline is drawn
/// as seen from that side.
pub fn plate_of(
    body: &Body,
    tolerance: Tolerance,
    name: &dyn Fn(FaceId) -> String,
) -> Result<Plate, NotPlate> {
    if body.kind != BodyKind::Solid {
        return Err(not(
            "it is a sheet without thickness, not a solid",
            Vec::new(),
        ));
    }
    let flats = flats(body);
    if flats.is_empty() {
        let mut kinds: Vec<&str> = body.faces.iter().map(|f| f.surface.kind()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let first: Vec<FaceId> = (0..body.faces.len().min(1))
            .filter_map(|i| u32::try_from(i).ok().map(FaceId))
            .collect();
        return Err(not(
            format!(
                "no face of it is flat (its faces are: {}): a plate has a flat top and bottom",
                kinds.join(", ")
            ),
            first,
        ));
    }
    let mut points: Vec<Vec3> = body.vertices.iter().map(|v| v.point).collect();
    if points.is_empty() {
        return Err(not("it has no vertices to measure", Vec::new()));
    }
    let size = {
        let Some(&first) = points.first() else {
            return Err(not("it has no vertices to measure", Vec::new()));
        };
        let (mut lo, mut hi) = (first, first);
        for p in &points {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (hi - lo).length()
    };
    if !size.is_finite() {
        return Err(not("a vertex of it is not finite", Vec::new()));
    }
    points.dedup();
    let near = 10.0 * tolerance.linear + 1e-12 * size;
    // The axes, each once whichever way its first face faces.
    let parallel = tolerance.angular.max(1e-9);
    let mut axes: Vec<Vec3> = Vec::new();
    for f in &flats {
        if axes.iter().any(|a| a.cross(f.normal).length() <= parallel) {
            continue;
        }
        if axes.len() >= MAX_AXES {
            return Err(not(
                format!("it has flat faces in more than {MAX_AXES} directions"),
                Vec::new(),
            ));
        }
        axes.push(f.normal);
    }
    // Each axis with flat faces on both sides at the body's extremes.
    let mut candidates: Vec<(f64, usize, Vec3, f64, f64)> = Vec::new();
    for (i, a) in axes.iter().enumerate() {
        let (lo, hi) = points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                let l = a.dot(*p);
                (lo.min(l), hi.max(l))
            });
        let on = |side: f64, level: f64| {
            flats.iter().any(|f| {
                f.normal.dot(*a) * side > 0.0
                    && f.normal.cross(*a).length() <= parallel
                    && (f.level * side - level).abs() <= near
            })
        };
        if hi - lo > near && on(1.0, hi) && on(-1.0, lo) {
            candidates.push((hi - lo, i, *a, lo, hi));
        }
    }
    candidates.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut first_fault: Option<(f64, NotPlate)> = None;
    for (t, _, axis, lo, hi) in candidates.iter().take(MAX_TESTED) {
        // The top faces most up: by z, then y, then x.
        let up = [axis.z, axis.y, axis.x]
            .into_iter()
            .find(|c| c.abs() > 1e-12)
            .unwrap_or(1.0);
        let (axis, lo, hi) = if up > 0.0 {
            (*axis, *lo, *hi)
        } else {
            (-*axis, -hi, -lo)
        };
        match plate_along(body, &flats, axis, (lo, hi), tolerance, near, name) {
            Ok(plate) => {
                return Ok(Plate {
                    thinner: first_fault,
                    ..plate
                });
            }
            Err(fault) => {
                first_fault.get_or_insert((*t, fault));
            }
        }
    }
    Err(first_fault.map(|(_, fault)| fault).unwrap_or_else(|| {
        let faces: Vec<FaceId> = flats.iter().take(1).map(|f| f.face).collect();
        not(
            format!(
                "no two flat faces face away from each other with the whole body between them (flat faces such as {}); a plate has a flat top and bottom",
                faces.iter().map(|f| name(*f)).collect::<Vec<_>>().join(", ")
            ),
            faces,
        )
    }))
}
