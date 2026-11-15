//! A tip: the flat end face of a body replaced by faces that close the
//! body in one point (docs/design/sweeps.md › *A tip on a body*).
//!
//! The faces are the nets of [`craft3d_geom::tip`]: one per edge of the
//! end face's loop, each running on from the side face behind that edge
//! without a crease and all reaching the point with one tangent plane,
//! parallel to the end. They are spliced in where the end face was: the
//! end face and its loop go, every other face, edge and vertex of the body
//! stays as it was under its own number, and the rim edges are now edges
//! between their side and a face of the tip. One new edge (a *rail*) runs
//! from every vertex of the loop to the tip, and the tip is one new vertex.
//!
//! **What is taken.** A flat face with one loop of two or more edges, each
//! a whole border of the spline surface of the one other face it bounds
//! (a loft's side, an extruded or revolved spline: a Bézier or B-spline
//! surface whose knot vectors are clamped). What is refused, each with the
//! face or the edge: a face that is not flat, a face with a hole, a loop of
//! one closed edge, a side that is an analytic surface (a plane, a
//! cylinder, a cone, …: their cross rows in closed form come later), an
//! edge that is not a whole border of its side's surface, and whatever the
//! nets refuse (a side that does not go on toward the tip, a tip that
//! stands outside what the sides are carried on to, sides that close in
//! before the tip; [`craft3d_geom::tip::TipError`]).
//!
//! **Where the tip stands.** `length` beyond the end along its outward
//! normal; by default (no length given) as long as the radius of the
//! circle with the end's area, and never more than half the length at
//! which the sides close in (`longest_tip_length`). Over the centre of
//! area of what the sides are carried on to in the tip's plane (the
//! shoulder), or over a point given in the end face's own axes.

use crate::KernelError;
use craft3d_geom::pole::Border;
use craft3d_geom::tip::{
    FULLNESS_RANGE, Tip, TipError, TipForm, TipRim, TipSpec, longest_tip_length, tip_nets,
    tip_shoulder,
};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, EdgeId, FaceId, LoopId, Shell, VertexId, edge_faces};

const OPERATION: &str = "tip";

/// The fullness of quarter ellipses across the tip, `√2 − 1` (the
/// conic's weight `√½`): the half ball on a rod as long as its radius, a
/// half ellipsoid on an elliptic prism. The default.
pub const ROUND_FULLNESS: f64 = std::f64::consts::SQRT_2 - 1.0;

/// The most edges an end face's loop may have (the nets' own cap).
pub const MAX_TIP_EDGES: usize = craft3d_geom::tip::MAX_TIP_RIMS;

/// What is asked of a tip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipAsk {
    /// The flat end face to close.
    pub face: FaceId,
    /// How far beyond the end the tip lies, along the end's outward
    /// normal; `None` for the length chosen as the module says.
    pub length: Option<f64>,
    /// Where the tip stands over the end, in the end face's own axes (the
    /// frame of its plane: its x and y axes from its origin); `None` for
    /// the centre of area of the shoulder.
    pub at: Option<[f64; 2]>,
    /// The conic's `ρ`, within [`FULLNESS_RANGE`]: [`ROUND_FULLNESS`]
    /// gives quarter ellipses, ½ parabolas, more is blunter.
    pub fullness: f64,
}

/// Where a face of a tipped body came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipOrigin {
    /// The face of the body given, unchanged.
    Kept(FaceId),
    /// A face of the tip: the one that continues this side face of the
    /// body given.
    Tip { side: FaceId },
}

/// A tipped body and where its entities came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Tipped {
    pub body: Body,
    /// Per face of `body`: the faces of the body given in their order,
    /// without the end face, then the faces of the tip in the order of
    /// the end face's loop.
    pub faces: Vec<TipOrigin>,
    /// Per edge of `body`: the edge of the body given it is (every edge
    /// of that body, under its own number), `None` for a rail.
    pub edges: Vec<Option<EdgeId>>,
    /// Per vertex of `body`: the vertex of the body given it is, `None`
    /// for the tip.
    pub vertices: Vec<Option<VertexId>>,
    /// The length used, beyond the end.
    pub length: f64,
    /// The tip point.
    pub tip: Vec3,
    pub form: TipForm,
    /// The least sine at which the sides leave the end toward the tip.
    pub onward: f64,
    /// A lower bound on the sine at which the rays from the tip cross the
    /// shoulder (0 where none was found): `Tip::star`.
    pub star: f64,
    /// The largest angle between the two faces of a rail, along it:
    /// nothing where every rail is smooth.
    pub crease: f64,
}

fn refused(faces: Vec<FaceId>, edge: Option<EdgeId>, reason: impl Into<String>) -> KernelError {
    KernelError::Tip {
        faces,
        edge,
        reason: reason.into(),
    }
}

/// The body with `ask.face` closed by a tip, or why not. A body with
/// loose entities runs at its working tolerance (`loose_op`).
pub fn tip(
    body: &Body,
    ask: &TipAsk,
    tolerance: &Tolerance,
    loosest: Option<f64>,
) -> Result<Tipped, KernelError> {
    crate::loose_op::run(
        OPERATION,
        body,
        tolerance,
        loosest,
        |at| tip_at(body, ask, at),
        |r| {
            let sources = crate::loose_op::Sources {
                faces: r
                    .faces
                    .iter()
                    .map(|origin| match origin {
                        TipOrigin::Kept(face) => Some(*face),
                        TipOrigin::Tip { .. } => None,
                    })
                    .collect(),
                edges: r.edges.clone(),
                vertices: r.vertices.clone(),
            };
            (&mut r.body, sources)
        },
    )
}

/// One edge of the end face's loop and the side behind it.
struct RimEdge {
    coedge: Coedge,
    side: FaceId,
    rim: TipRim,
}

/// The operation itself, at the tolerance given.
fn tip_at(body: &Body, ask: &TipAsk, tolerance: &Tolerance) -> Result<Tipped, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let (least, most) = FULLNESS_RANGE;
    if !(ask.fullness >= least && ask.fullness <= most) {
        return Err(KernelError::InvalidSize {
            name: "fullness",
            value: ask.fullness,
        });
    }
    if let Some(length) = ask.length
        && !(length.is_finite() && length > 0.0)
    {
        return Err(KernelError::InvalidSize {
            name: "length",
            value: length,
        });
    }
    if let Some([x, y]) = ask.at
        && !(x.is_finite() && y.is_finite())
    {
        return Err(KernelError::InvalidSize {
            name: "at",
            value: if x.is_finite() { y } else { x },
        });
    }
    let end = ask.face;
    let face = body
        .face(end)
        .ok_or_else(|| refused(vec![], None, "there is no such face in the body"))?;
    let Surface::Plane(plane) = &face.surface else {
        return Err(refused(
            vec![end],
            None,
            format!(
                "the face is a {}, not flat: a tip closes a flat end face",
                face.surface.kind()
            ),
        ));
    };
    let frame = *plane.frame();
    let outward = if face.reversed { -frame.z() } else { frame.z() };
    let [outer] = face.loops.as_slice() else {
        return Err(refused(
            vec![end],
            None,
            "the face has a hole (more than one loop): a tip over a ring is not built yet",
        ));
    };
    let outer = *outer;
    let coedges = body
        .loop_(outer)
        .map(|l| l.coedges.clone())
        .ok_or_else(|| refused(vec![end], None, "the face's loop is missing"))?;
    if coedges.len() < 2 {
        return Err(refused(
            vec![end],
            coedges.first().map(|c| c.edge),
            "the face is bounded by one closed edge: a tip over such a rim is not built yet (its side would have to be cut in two)",
        ));
    }
    if coedges.len() > MAX_TIP_EDGES {
        return Err(refused(
            vec![end],
            None,
            format!(
                "the face's loop has {} edges; a tip takes {MAX_TIP_EDGES} at most",
                coedges.len()
            ),
        ));
    }
    let users = edge_faces(body);
    let mut rims: Vec<RimEdge> = Vec::with_capacity(coedges.len());
    for &coedge in &coedges {
        let rim = rim_edge(body, &users, end, coedge, plane.frame(), tolerance)?;
        if rims.iter().any(|other| other.side == rim.side) {
            return Err(refused(
                vec![end, rim.side],
                Some(coedge.edge),
                "two edges of the end face lie on one side face: a tip takes a side face along one whole border",
            ));
        }
        rims.push(rim);
    }
    let tip_rims: Vec<TipRim> = rims.iter().map(|r| r.rim.clone()).collect();
    let rim_curves: Vec<&NurbsCurve> = tip_rims.iter().map(|r| &r.curve).collect();
    let (area, centre) = area_and_centre(&rim_curves, outward).ok_or_else(|| {
        refused(
            vec![end],
            None,
            "the face encloses no area seen along its normal",
        )
    })?;
    let foot = match ask.at {
        Some([x, y]) => {
            // On the end's plane, whatever the frame's origin.
            let point = frame.point(x, y, 0.0);
            point - outward * (point - centre).dot(outward)
        }
        None => centre,
    };
    let edge_of = |rim: usize| rims.get(rim).map(|r| r.coedge.edge);
    let named = |error: TipError| tip_refusal(end, error, &edge_of);
    let length = match ask.length {
        Some(length) => length,
        None => {
            automatic_length(&tip_rims, foot, outward, ask.fullness, area, tol).map_err(named)?
        }
    };
    let plane_point = centre + outward * length;
    let tip_point = match ask.at {
        Some(_) => foot + outward * length,
        None => {
            let (_, shoulder) =
                tip_shoulder(&tip_rims, plane_point, outward, tol).map_err(named)?;
            let shoulder: Vec<&NurbsCurve> = shoulder.iter().collect();
            let (_, middle) = area_and_centre(&shoulder, outward).ok_or_else(|| {
                refused(
                    vec![end],
                    None,
                    "what the sides are carried on to in the tip's plane encloses no area: they close in before the tip; give a shorter length",
                )
            })?;
            // In the plane, to the bit of its offset along the normal.
            middle + outward * (plane_point - middle).dot(outward)
        }
    };
    let spec = TipSpec {
        tip: tip_point,
        normal: outward,
        fullness: ask.fullness,
        tolerance: tol,
        angular: tolerance.angular,
    };
    let made = tip_nets(&tip_rims, &spec).map_err(named)?;
    if !made.counter_clockwise {
        return Err(refused(
            vec![end],
            None,
            "the face's loop does not run round its outward normal as a face's outer loop does",
        ));
    }
    if made.faces.len() != rims.len() || made.rails.len() != rims.len() {
        return Err(refused(
            vec![end],
            None,
            craft3d_math::bug_text!("the tip has not one face per edge of the end"),
        ));
    }
    for (index, &off) in made.off_sides.iter().enumerate() {
        if off.is_nan() || off > tolerance.angular {
            return Err(refused(
                vec![end],
                edge_of(index),
                format!(
                    "the sides behind this edge and the next agree where they meet only within the tolerance, and the tip's face would leave its side by {off:e} rad there, more than the angular tolerance; make the sides meet exactly"
                ),
            ));
        }
    }
    let sides: Vec<FaceId> = rims.iter().map(|r| r.side).collect();
    let (spliced, faces, edges, vertices) = splice(body, end, outer, &coedges, &sides, &made)?;
    let mut defects = craft3d_topo::check(&spliced, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        });
    }
    let crease = made
        .creases
        .iter()
        .map(|c| c.largest.max(c.at_rim))
        .fold(0.0_f64, f64::max);
    Ok(Tipped {
        body: spliced,
        faces,
        edges,
        vertices,
        length,
        tip: made.tip,
        form: made.form,
        onward: made.onward,
        star: made.star,
        crease,
    })
}

/// A refusal of the nets, with the edge of the rim curve it names.
fn tip_refusal(
    end: FaceId,
    error: TipError,
    edge_of: &dyn Fn(usize) -> Option<EdgeId>,
) -> KernelError {
    let rim = match &error {
        TipError::RimOpen { rim, .. }
        | TipError::RimCrosses { rim, .. }
        | TipError::SidesDisagree { rim, .. }
        | TipError::RimReachesPlane { rim, .. }
        | TipError::NotOnward { rim, .. }
        | TipError::NotStarShaped { rim, .. }
        | TipError::ClosesIn { rim, .. }
        | TipError::NotProven { rim, .. } => Some(*rim),
        TipError::Invalid(_)
        | TipError::Unsupported(_)
        | TipError::Geometry(_)
        | TipError::NoLengthFits => None,
    };
    let advice = match &error {
        TipError::NotStarShaped { .. } | TipError::ClosesIn { .. } | TipError::NoLengthFits => {
            "; give a shorter length, or a place for the tip further inside the end"
        }
        TipError::NotProven { .. } => {
            "; give a place for the tip (at) further from the end's edges, seen from where the whole end is in view"
        }
        TipError::RimReachesPlane { .. } => "; give a longer length",
        TipError::NotOnward { .. } => {
            "; the faces round the end must leave it toward the tip, not along it or back"
        }
        _ => "",
    };
    refused(vec![end], rim.and_then(edge_of), format!("{error}{advice}"))
}

/// The edge of `coedge` (a coedge of the end face's loop) and the side
/// face behind it, with the rim curve running the way the loop runs and
/// the side's cross row.
fn rim_edge(
    body: &Body,
    users: &[Vec<FaceId>],
    end: FaceId,
    coedge: Coedge,
    frame: &craft3d_math::Frame,
    tolerance: &Tolerance,
) -> Result<RimEdge, KernelError> {
    let tol = tolerance.linear;
    let edge_id = coedge.edge;
    let here = |reason: &str| refused(vec![end], Some(edge_id), reason.to_string());
    let edge = body
        .edge(edge_id)
        .ok_or_else(|| here("the loop names an edge the body does not have"))?;
    let faces = users
        .get(edge_id.0 as usize)
        .ok_or_else(|| here("the loop names an edge the body does not have"))?;
    let others: Vec<FaceId> = faces.iter().copied().filter(|f| *f != end).collect();
    let [side] = others.as_slice() else {
        return Err(here(
            "the edge does not bound exactly one other face: the end face must be bounded by the faces round it",
        ));
    };
    let side = *side;
    let at_side = |reason: String| refused(vec![end, side], Some(edge_id), reason);
    let surface = match &body.face(side).map(|f| &f.surface) {
        Some(Surface::Nurbs(surface)) => surface.clone(),
        Some(Surface::Bezier(patch)) => {
            NurbsSurface::from_bezier(patch).map_err(KernelError::Geometry)?
        }
        Some(other) => {
            return Err(at_side(format!(
                "the face behind this edge is a {}: a tip over sides that are planes, cylinders, cones, spheres or tori is not built yet (only over spline sides: a loft's, a swept or turned spline's)",
                other.kind()
            )));
        }
        None => return Err(here("the face behind this edge is missing")),
    };
    let (start, finish) = body
        .coedge_ends(coedge)
        .and_then(|(a, b)| Some((body.vertex(a)?.point, body.vertex(b)?.point)))
        .ok_or_else(|| here("the edge's vertices are missing"))?;
    let normal = frame.z();
    let origin = frame.origin();
    let mut found: Option<TipRim> = None;
    for border in [Border::UStart, Border::UEnd, Border::VStart, Border::VEnd] {
        let Ok(rim) = TipRim::of_border(&surface, border) else {
            continue;
        };
        let points = rim.curve.control_points();
        let in_plane = points
            .iter()
            .all(|p| (*p - origin).dot(normal).abs() <= tol);
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            continue;
        };
        if !in_plane {
            continue;
        }
        let rim = if first.distance(start) <= tol && last.distance(finish) <= tol {
            rim
        } else if first.distance(finish) <= tol && last.distance(start) <= tol {
            match rim.reversed() {
                Ok(rim) => rim,
                Err(_) => continue,
            }
        } else {
            continue;
        };
        if !along_the_edge(&rim.curve, &edge.curve, tol) {
            continue;
        }
        if found.is_some() {
            return Err(at_side(
                "two borders of the face behind this edge run along it: its surface is folded flat there".to_string(),
            ));
        }
        found = Some(rim);
    }
    let rim = found.ok_or_else(|| {
        at_side(
            "the edge is not a whole border of the spline surface of the face behind it (that face was cut there, or its surface runs on past the end): a tip over such a side is not built yet".to_string(),
        )
    })?;
    Ok(RimEdge { coedge, side, rim })
}

/// Whether `rim` runs along `edge`: points at a quarter, a half and three
/// quarters of its range within twice the tolerance of the edge's curve.
fn along_the_edge(rim: &NurbsCurve, edge: &Curve3, tol: f64) -> bool {
    let (lo, hi) = rim.domain();
    [0.25, 0.5, 0.75].iter().all(|share| {
        let point = rim.eval(lo + (hi - lo) * share);
        edge.project(point)
            .map(|t| edge.eval(t).distance(point) <= 2.0 * tol)
            .unwrap_or(false)
    })
}

/// The area the closed chain of `curves` encloses seen along `normal`
/// (positive where it runs counter-clockwise), and its centre of area,
/// by Green's theorem with Gauss–Legendre (16 points on each quarter of
/// every knot span: a rational span is no polynomial): each
/// sliver between the first point and a step along the curve is a
/// triangle of area `½ n · (r × dr)` whose centre is two thirds of the
/// way out. `None` where it encloses nothing.
fn area_and_centre(curves: &[&NurbsCurve], normal: Vec3) -> Option<(f64, Vec3)> {
    let rule = Rule::new(16);
    let about = *curves.first()?.control_points().first()?;
    let mut area = 0.0_f64;
    let mut moment = Vec3::new(0.0, 0.0, 0.0);
    let mut size = 0.0_f64;
    for curve in curves {
        for p in curve.control_points() {
            size = size.max(p.distance(about));
        }
        let knots = curve.knots();
        for pair in knots.windows(2) {
            let [lo, hi] = pair else { continue };
            if hi <= lo {
                continue;
            }
            let quarter = 0.25 * (hi - lo);
            for panel in 0..4u8 {
                let from = lo + quarter * f64::from(panel);
                for (t, weight) in rule.points(from, from + quarter) {
                    let (point, along) = curve.derivatives(t);
                    let r = point - about;
                    let sliver = 0.5 * weight * normal.dot(r.cross(along));
                    area += sliver;
                    moment += r * (sliver * 2.0 / 3.0);
                }
            }
        }
    }
    let least = 1e-12 * size * size;
    if !area.is_finite() || area.abs() <= least {
        return None;
    }
    let centre = about + moment * (1.0 / area);
    centre.is_finite().then_some((area, centre))
}

/// The default length: the radius of the circle with the end's `area`,
/// and never more than half the length at which the sides close in
/// (looked for up to twice that radius).
fn automatic_length(
    rims: &[TipRim],
    foot: Vec3,
    normal: Vec3,
    fullness: f64,
    area: f64,
    tol: f64,
) -> Result<f64, TipError> {
    let radius = (area.abs() / std::f64::consts::PI).sqrt();
    if !(radius.is_finite() && radius > 4.0 * tol) {
        return Err(TipError::Invalid(
            "the end is too small for a tip at this tolerance",
        ));
    }
    let found = longest_tip_length(rims, foot, normal, fullness, tol, 2.0 * radius)?;
    Ok(match found.fails {
        None => radius,
        Some(_) => radius.min(0.5 * found.fits),
    })
}

/// The new loops, faces and shells, the rails and the tip vertex.
type Spliced = (
    Body,
    Vec<TipOrigin>,
    Vec<Option<EdgeId>>,
    Vec<Option<VertexId>>,
);

/// The body with the end face `end` (whose loop is `outer`, of
/// `coedges`) taken out and the faces of `made` put in its place, each
/// bounded by its rim edge (used as the end face used it), the rail of
/// the next edge and its own.
fn splice(
    body: &Body,
    end: FaceId,
    outer: LoopId,
    coedges: &[Coedge],
    sides: &[FaceId],
    made: &Tip,
) -> Result<Spliced, KernelError> {
    let lost = || {
        refused(
            vec![end],
            None,
            craft3d_math::bug_text!("the tip lost track of the body's entities"),
        )
    };
    let mut out = Body::new(body.kind);
    out.vertices = body.vertices.clone();
    out.edges = body.edges.clone();
    out.tolerances = body.tolerances.clone();
    let vertices: Vec<Option<VertexId>> = (0..body.vertices.len())
        .map(|i| u32::try_from(i).ok().map(VertexId))
        .chain(std::iter::once(None))
        .collect();
    let top = out.add_vertex(made.tip);
    let count = coedges.len();
    let mut rails = Vec::with_capacity(count);
    for (coedge, rail) in coedges.iter().zip(&made.rails) {
        let (from, _) = body.coedge_ends(*coedge).ok_or_else(lost)?;
        rails.push(out.add_edge(Curve3::Nurbs(rail.clone()), rail.domain(), from, top));
    }
    let edges: Vec<Option<EdgeId>> = (0..body.edges.len())
        .map(|i| u32::try_from(i).ok().map(EdgeId))
        .chain(std::iter::repeat_n(None, count))
        .collect();
    // The loops in their order, without the end face's.
    let mut loop_ids: Vec<Option<LoopId>> = Vec::with_capacity(body.loops.len());
    for (index, l) in body.loops.iter().enumerate() {
        if u32::try_from(index).ok() == Some(outer.0) {
            loop_ids.push(None);
        } else {
            loop_ids.push(Some(out.add_loop(l.coedges.clone())));
        }
    }
    let mut face_ids: Vec<Option<FaceId>> = Vec::with_capacity(body.faces.len());
    let mut faces: Vec<TipOrigin> = Vec::with_capacity(body.faces.len() + count);
    for (index, face) in body.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(index).map_err(|_| lost())?);
        if id == end {
            face_ids.push(None);
            continue;
        }
        let loops = face
            .loops
            .iter()
            .map(|l| loop_ids.get(l.0 as usize).copied().flatten())
            .collect::<Option<Vec<LoopId>>>()
            .ok_or_else(lost)?;
        face_ids.push(Some(out.add_face(
            face.surface.clone(),
            face.reversed,
            loops,
        )));
        faces.push(TipOrigin::Kept(id));
    }
    let rail = |k: usize| rails.get(k % count).copied().ok_or_else(lost);
    let mut tip_faces = Vec::with_capacity(count);
    for (k, ((coedge, surface), side)) in coedges.iter().zip(&made.faces).zip(sides).enumerate() {
        // Round the face counter-clockwise seen from outside: along the
        // rim as the end face ran along it, out along the next rail, back
        // along its own.
        let outline = out.add_loop(vec![
            *coedge,
            Coedge {
                edge: rail(k + 1)?,
                reversed: false,
            },
            Coedge {
                edge: rail(k)?,
                reversed: true,
            },
        ]);
        tip_faces.push(out.add_face(Surface::Nurbs(surface.clone()), false, vec![outline]));
        faces.push(TipOrigin::Tip { side: *side });
    }
    for shell in &body.shells {
        let mut list = Vec::with_capacity(shell.faces.len() + count);
        for face in &shell.faces {
            if *face == end {
                list.extend(tip_faces.iter().copied());
            } else {
                list.push(
                    face_ids
                        .get(face.0 as usize)
                        .copied()
                        .flatten()
                        .ok_or_else(lost)?,
                );
            }
        }
        out.shells.push(Shell { faces: list });
    }
    Ok((out, faces, edges, vertices))
}
