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
//! which the sides close in (`longest_tip_length`). Over a place given in
//! the end face's own axes or as a point in the model; by default over
//! the centre of the taper (the place on the end the sides close in on,
//! [`TipPlaced::Taper`]) where the sides narrow toward one that sees the
//! whole end, else over the centre of area of what the sides are carried
//! on to in the tip's plane (the shoulder), and where that does not see
//! the whole end, over the middle of the places that do.

use crate::KernelError;
use craft3d_geom::pole::Border;
use craft3d_geom::tip::{
    Tip, TipError, TipForm, TipRim, TipSpec, longest_tip_length, tip_nets, tip_shoulder,
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

/// The least and the largest fullness a tip takes: narrower than the
/// nets' own range (`FULLNESS_RANGE`, 0.001 to 0.999), to what is built.
/// Measured (2026-11-16) over lofted circles, ovals, a hull and a lens at
/// a hundredth to twice the radius of the end's area: 0.99 was refused on
/// every one (the self-intersection check cannot mesh it), 0.98 and
/// 0.01 to 0.02 often; 0.05 to 0.95 built on all of them from a tenth of
/// that radius.
pub const FULLNESS: (f64, f64) = (0.01, 0.98);

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
    /// Where the tip stands over the end; `None` for the place chosen as
    /// the module says.
    pub at: Option<TipPlace>,
    /// The conic's `ρ`, within [`FULLNESS`]: [`ROUND_FULLNESS`]
    /// gives quarter ellipses, ½ parabolas, more is blunter.
    pub fullness: f64,
}

/// Where a tip is asked to stand over its end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TipPlace {
    /// Lengths along the end face's own x and y axes (the frame of its
    /// plane) from its plane's origin.
    Face([f64; 2]),
    /// A point in the model, taken to the end's plane along the end's
    /// normal. It must lie between the end's plane and the tip's, within
    /// the tolerance: on the end, at the tip itself, or between.
    Point(Vec3),
}

/// How the place of a tip was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipPlaced {
    /// Given in the end face's own axes.
    Face,
    /// Given as a point in the model.
    Point,
    /// Chosen: the centre of the taper, the place on the end that the
    /// sides, carried on, close in on (the foot of a cone's apex; the
    /// line a loft's sections are drawn and narrowed about). Where the
    /// sides narrow, a tip there continues the body's own axis.
    Taper,
    /// Chosen: the centre of area of what the sides are carried on to in
    /// the tip's plane (the shoulder), where they do not narrow toward a
    /// place that sees the whole end (a prism, a body that leans more than
    /// it narrows).
    Shoulder,
    /// Chosen: the middle of the places that see the whole end, where the
    /// end's centre of area does not (an end that is not convex, an L).
    InView,
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
    /// The end face's own axes, in which a place is given (`TipPlace::Face`):
    /// the frame of its plane, as the face's surface has it.
    pub frame: craft3d_math::Frame,
    /// The end's outward normal: the frame's z axis, or against it where
    /// the face is reversed.
    pub normal: Vec3,
    /// The tip's foot: the point of the end's plane it stands over.
    pub foot: Vec3,
    /// The foot in the end face's own axes: what `TipPlace::Face` gives it
    /// as.
    pub at: [f64; 2],
    /// How the place was found.
    pub placed: TipPlaced,
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
    refused_as("face", faces, edge, reason)
}

/// [`refused`], blaming another input than the face.
fn refused_as(
    blame: &'static str,
    faces: Vec<FaceId>,
    edge: Option<EdgeId>,
    reason: impl Into<String>,
) -> KernelError {
    KernelError::Tip {
        faces,
        edge,
        blame,
        reason: reason.into(),
        longest: None,
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
    /// The side's surface, the border the rim lies on, and whether the
    /// rim runs against that border's parameter.
    surface: NurbsSurface,
    border: Border,
    against: bool,
}

/// The operation itself, at the tolerance given. A tip at the centre of
/// the taper that is refused is tried once more where the place was
/// chosen before there was one (over the shoulder's centre of area), so
/// the default place never refuses what that one built.
fn tip_at(body: &Body, ask: &TipAsk, tolerance: &Tolerance) -> Result<Tipped, KernelError> {
    let tapered = std::cell::Cell::new(false);
    match tip_placed(body, ask, tolerance, true, &tapered) {
        Err(_) if tapered.get() => tip_placed(body, ask, tolerance, false, &tapered),
        made => made,
    }
}

/// [`tip_at`], the centre of the taper tried for a place left to choose
/// when `taper` (and `tapered` set where it is taken).
fn tip_placed(
    body: &Body,
    ask: &TipAsk,
    tolerance: &Tolerance,
    taper: bool,
    tapered: &std::cell::Cell<bool>,
) -> Result<Tipped, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let (least, most) = FULLNESS;
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
    let given = match ask.at {
        Some(TipPlace::Face([x, y])) => [x, y, 0.0],
        Some(TipPlace::Point(p)) => [p.x, p.y, p.z],
        None => [0.0; 3],
    };
    if let Some(&bad) = given.iter().find(|x| !x.is_finite()) {
        return Err(KernelError::InvalidSize {
            name: "at",
            value: bad,
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
    let edge_of = |rim: usize| rims.get(rim).map(|r| r.coedge.edge);
    // The end's plane seen from outside, counter-clockwise.
    let in_the_plane = if frame.z().dot(outward) > 0.0 {
        (frame.x(), frame.y())
    } else {
        (frame.x(), -frame.y())
    };
    // Which places see the whole end: worked out only when asked.
    let sight = std::cell::OnceCell::new();
    let sight = || sight.get_or_init(|| Sight::of(&tip_rims, in_the_plane, centre));
    // On the end's plane, whatever the frame's origin.
    let on_plane = |p: Vec3| p - outward * (p - centre).dot(outward);
    let chosen = match ask.at {
        Some(TipPlace::Face([x, y])) => Some((on_plane(frame.point(x, y, 0.0)), TipPlaced::Face)),
        Some(TipPlace::Point(p)) => Some((on_plane(p), TipPlaced::Point)),
        // Where the sides narrow toward a place that sees the whole end
        // (as far as the places the sight was cut by show).
        None if taper => taper_centre(
            &tip_rims,
            centre,
            outward,
            in_the_plane,
            ask.length
                .unwrap_or_else(|| (area.abs() / std::f64::consts::PI).sqrt()),
            tol,
        )
        .filter(|&f| sight().known() && sight().sees(f) == Seen::Maybe)
        .map(|f| {
            tapered.set(true);
            (f, TipPlaced::Taper)
        }),
        None => None,
    };
    // Otherwise the length is looked for over the end's centre of area
    // and the tip stands over the shoulder's, or where the end's does not
    // see the whole end, both over the middle of the places that do.
    let (foot, placed) = match chosen {
        Some(found) => found,
        None => match sight().sees(centre) {
            Seen::NotFrom => sight()
                .middle()
                .map_or((centre, TipPlaced::Shoulder), |m| (m, TipPlaced::InView)),
            Seen::Nowhere | Seen::Maybe => (centre, TipPlaced::Shoulder),
        },
    };
    let at_given = ask.at.is_some();
    let seen = || sight().sees(foot);
    let named = |error: TipError| tip_refusal(end, error, &edge_of, &seen, at_given, None);
    let length = match ask.length {
        Some(length) => length,
        None => {
            automatic_length(&tip_rims, foot, outward, ask.fullness, area, tol).map_err(named)?
        }
    };
    // A point in the model lies between the end's plane and the tip's.
    if let Some(TipPlace::Point(p)) = ask.at {
        let off = (p - centre).dot(outward);
        if !(off >= -tol && off <= length + tol) {
            let said = if off < 0.0 {
                format!("{:e} behind the end's plane", -off)
            } else {
                format!("{off:e} beyond the end's plane, past the tip's ({length:e} beyond it)")
            };
            return Err(refused_as(
                "at",
                vec![end],
                None,
                format!(
                    "the point given for the tip lies {said}: give a point on the end, at the tip, or between the two (it is taken to the end's plane along the end's normal)"
                ),
            ));
        }
    }
    let plane_point = centre + outward * length;
    let tip_point = match placed {
        TipPlaced::Shoulder => {
            let (_, shoulder) =
                tip_shoulder(&tip_rims, plane_point, outward, tol).map_err(named)?;
            let shoulder: Vec<&NurbsCurve> = shoulder.iter().collect();
            let (_, middle) = area_and_centre(&shoulder, outward).ok_or_else(|| {
                refused_as(
                    "length",
                    vec![end],
                    None,
                    "what the sides are carried on to in the tip's plane encloses no area: they close in before the tip; give a shorter length",
                )
            })?;
            // In the plane, to the bit of its offset along the normal.
            middle + outward * (plane_point - middle).dot(outward)
        }
        TipPlaced::Face | TipPlaced::Point | TipPlaced::Taper | TipPlaced::InView => {
            foot + outward * length
        }
    };
    let foot = on_plane(tip_point);
    // Where a length given is too long, the longest that fits over the
    // same place is looked for and said.
    let longest = || {
        ask.length?;
        longest_tip_length(&tip_rims, foot, outward, ask.fullness, tol, length)
            .ok()
            .map(|found| found.fits)
    };
    let named =
        |error: TipError| tip_refusal(end, error, &edge_of, &seen, at_given, Some(&longest));
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
    // Tangent to the sides along the rim, held on the faces made: the
    // nets' own condition evaluated on their stored control points, with
    // what rounding can add.
    for (index, (face, rim)) in made.faces.iter().zip(&rims).enumerate() {
        let side_reversed = body.face(rim.side).is_some_and(|f| f.reversed);
        let off = rim_angle(face, rim, side_reversed);
        if off.is_nan() || off > tolerance.angular {
            let at_least = "; give a longer length";
            return Err(refused_as(
                "length",
                vec![end],
                edge_of(index),
                format!(
                    "at this length the tip's face is not tangent to the side along this edge within the angular tolerance: it leaves it by {off:e} rad, against {:e} (a tip a few tolerances long, or very short against the size of the end and its distance from the origin){at_least}",
                    tolerance.angular
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
    // Every face of the tip can be measured (the integrals a body's
    // volume, area and centroid sum), or the tip is refused: a body that
    // cannot be measured cannot be checked, exported or trusted.
    if let Some((index, error)) = unmeasured(&spliced, &faces) {
        return Err(refused_as(
            "length",
            vec![end],
            edge_of(index),
            format!(
                "the tip's face along this edge cannot be measured ({error}); make the tip longer, or its fullness nearer the middle (0.4142, quarter ellipses)"
            ),
        ));
    }
    let crease = made
        .creases
        .iter()
        .map(|c| c.largest.max(c.at_rim))
        .fold(0.0_f64, f64::max);
    let from_origin = foot - frame.origin();
    Ok(Tipped {
        body: spliced,
        faces,
        edges,
        vertices,
        length,
        tip: made.tip,
        frame,
        normal: outward,
        foot,
        at: [from_origin.dot(frame.x()), from_origin.dot(frame.y())],
        placed,
        form: made.form,
        onward: made.onward,
        star: made.star,
        crease,
    })
}

/// Places along every knot span of a rim curve where the tip's face and
/// its side are compared.
const RIM_PLACES: u32 = 32;

/// The largest angle between the outward normals of the tip's `face`
/// and of its side (`rim`'s surface, `side_reversed` its face's flag) at
/// the rim, from the two surfaces' own derivatives at the same point of
/// the rim: at both ends and 31 places between of every knot span. What
/// is measured on the faces made, at those places: tangency between them
/// rests on the nets' construction (the face's row beside the rim is one
/// multiple of the side's step), this check on what was stored and how it
/// evaluates, rounding and all. Infinity where a normal cannot be had.
fn rim_angle(face: &NurbsSurface, rim: &RimEdge, side_reversed: bool) -> f64 {
    let ((_, _), (v0, _)) = face.domain();
    let curve = &rim.rim.curve;
    let knots = curve.knots();
    let (Some(&first), Some(&last)) = (knots.first(), knots.last()) else {
        return f64::INFINITY;
    };
    let ((su0, su1), (sv0, sv1)) = rim.surface.domain();
    let mut worst = 0.0_f64;
    for pair in knots.windows(2) {
        let [lo, hi] = pair else { continue };
        if hi <= lo {
            continue;
        }
        for i in 0..=RIM_PLACES {
            let t = lo + (hi - lo) * f64::from(i) / f64::from(RIM_PLACES);
            // The rim's parameter is the border's, or runs against it.
            let along = if rim.against { first + last - t } else { t };
            let (su, sv) = match rim.border {
                Border::UStart => (su0, along),
                Border::UEnd => (su1, along),
                Border::VStart => (along, sv0),
                Border::VEnd => (along, sv1),
            };
            let (_, tu, tv) = face.derivatives(t, v0);
            let (_, du, dv) = rim.surface.derivatives(su, sv);
            let (Some(tip_normal), Some(side_normal)) =
                (tu.cross(tv).normalized(), du.cross(dv).normalized())
            else {
                return f64::INFINITY;
            };
            let side_normal = if side_reversed {
                -side_normal
            } else {
                side_normal
            };
            let angle = tip_normal.angle_to(side_normal);
            if angle.is_nan() {
                return f64::INFINITY;
            }
            worst = worst.max(angle);
        }
    }
    worst
}

/// The first face of the tip (counted along the rim) that cannot be
/// measured, with why: each face integrated as `topo::mass_properties`
/// integrates it for the body's volume, area and centroid, about the same
/// middle and to the same size (a sheet of that face alone with every
/// vertex and edge of the body, which is what the size and the middle are
/// taken from). One face at a time, stopping at the first that fails: a
/// face whose integrals do not settle takes long to say so, and one is
/// enough.
fn unmeasured(body: &Body, faces: &[TipOrigin]) -> Option<(usize, String)> {
    let mut sheet = Body::new(craft3d_topo::BodyKind::Sheet);
    sheet.vertices = body.vertices.clone();
    sheet.edges = body.edges.clone();
    sheet.tolerances = body.tolerances.clone();
    let mut counted = 0usize;
    for (index, origin) in faces.iter().enumerate() {
        if !matches!(origin, TipOrigin::Tip { .. }) {
            continue;
        }
        let k = counted;
        counted += 1;
        let Some(face) = body.faces.get(index) else {
            return Some((k, "the face is missing".to_string()));
        };
        sheet.loops.clear();
        sheet.faces.clear();
        sheet.shells.clear();
        let mut loops = Vec::with_capacity(face.loops.len());
        for l in &face.loops {
            let Some(l) = body.loop_(*l) else {
                return Some((k, "the face's loop is missing".to_string()));
            };
            loops.push(sheet.add_loop(l.coedges.clone()));
        }
        let only = sheet.add_face(face.surface.clone(), face.reversed, loops);
        sheet.add_shell(vec![only]);
        if let Some(Err(error)) = craft3d_topo::face_areas(&sheet).into_iter().next() {
            return Some((k, error.to_string()));
        }
    }
    None
}

/// Whether a place sees the whole outline of the end: no point does (the
/// end is not star-shaped), the place does not, or it may.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Nowhere,
    NotFrom,
    Maybe,
}

/// The most half-planes [`Sight`] clips by.
const MAX_SIGHT_LINES: usize = 4096;

/// A line `(point, direction)` in the end's plane, in flat coordinates.
type Line = ([f64; 2], [f64; 2]);

/// Which places of the end's plane see the whole outline of the end made
/// of `rims`, read in the end's plane with the axes `axes`
/// (counter-clockwise seen from outside) about `centre`. A point sees a
/// closed curve whole only from the inner side of every tangent of it;
/// the polygon of points on the inner side of the tangents at some places
/// along the curve (a box cut by each, 32 places a knot span) holds every
/// point that sees it whole. So where that polygon is empty no point sees
/// the curve whole, and a foot outside it does not: both are proven by
/// the places taken. A foot inside may or may not.
struct Sight {
    axes: (Vec3, Vec3),
    centre: Vec3,
    lines: Vec<Line>,
    reach: f64,
    polygon: Polygon,
}

/// The polygon a [`Sight`] cut.
enum Polygon {
    /// Not cut: too many lines, or an outline of no size.
    Unknown,
    /// Empty: no point sees the whole outline.
    Empty,
    Cut(Vec<[f64; 2]>),
}

/// How far `q` lies to the left of `line` (its direction of travel), times
/// the direction's length.
fn left_of(q: [f64; 2], (p, d): Line) -> f64 {
    d[0] * (q[1] - p[1]) - d[1] * (q[0] - p[0])
}

impl Sight {
    fn of(rims: &[TipRim], axes: (Vec3, Vec3), centre: Vec3) -> Sight {
        let (ex, ey) = axes;
        let flat = |p: Vec3| [(p - centre).dot(ex), (p - centre).dot(ey)];
        let mut lines: Vec<Line> = Vec::new();
        let mut reach = 0.0_f64;
        for rim in rims {
            let curve = &rim.curve;
            for p in curve.control_points() {
                let [x, y] = flat(*p);
                reach = reach.max(x.abs()).max(y.abs());
            }
            for pair in curve.knots().windows(2) {
                let [lo, hi] = pair else { continue };
                if hi <= lo {
                    continue;
                }
                for i in 0..=32u8 {
                    let t = lo + (hi - lo) * f64::from(i) / 32.0;
                    let (p, d) = curve.derivatives(t);
                    let along = [d.dot(ex), d.dot(ey)];
                    if along[0] != 0.0 || along[1] != 0.0 {
                        lines.push((flat(p), along));
                    }
                }
            }
        }
        let polygon = Sight::cut(&lines, reach);
        Sight {
            axes,
            centre,
            lines,
            reach,
            polygon,
        }
    }

    fn cut(lines: &[Line], reach: f64) -> Polygon {
        if !(reach.is_finite() && reach > 0.0) || lines.len() > MAX_SIGHT_LINES {
            return Polygon::Unknown;
        }
        let big = 4.0 * reach;
        let mut polygon: Vec<[f64; 2]> = vec![[-big, -big], [big, -big], [big, big], [-big, big]];
        // Inside: to the left of the direction of travel.
        for &line in lines {
            let mut next = Vec::with_capacity(polygon.len() + 1);
            for (k, &a) in polygon.iter().enumerate() {
                let Some(&b) = polygon.get((k + 1) % polygon.len()) else {
                    continue;
                };
                let (sa, sb) = (left_of(a, line), left_of(b, line));
                if sa >= 0.0 {
                    next.push(a);
                }
                if (sa >= 0.0) != (sb >= 0.0) {
                    let t = sa / (sa - sb);
                    next.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                }
            }
            polygon = next;
            if polygon.len() < 3 {
                return Polygon::Empty;
            }
        }
        let area: f64 = polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
            .sum();
        if area.abs() <= 1e-12 * reach * reach {
            return Polygon::Empty;
        }
        Polygon::Cut(polygon)
    }

    fn flat(&self, p: Vec3) -> [f64; 2] {
        let (ex, ey) = self.axes;
        [(p - self.centre).dot(ex), (p - self.centre).dot(ey)]
    }

    /// Whether the polygon was cut: the outline is not too long to.
    fn known(&self) -> bool {
        !matches!(self.polygon, Polygon::Unknown)
    }

    /// Whether `foot` sees the whole outline.
    fn sees(&self, foot: Vec3) -> Seen {
        match self.polygon {
            Polygon::Unknown => Seen::Maybe,
            Polygon::Empty => Seen::Nowhere,
            Polygon::Cut(_) => {
                // Not inside by more than rounding: seen edge-on at best.
                let f = self.flat(foot);
                let margin = 1e-9 * self.reach;
                if self.lines.iter().any(|&(p, d)| {
                    let length = (d[0] * d[0] + d[1] * d[1]).sqrt();
                    left_of(f, (p, d)) <= margin * length
                }) {
                    Seen::NotFrom
                } else {
                    Seen::Maybe
                }
            }
        }
    }

    /// The centre of area of the polygon of the places that may see the
    /// whole outline, on the end's plane: the place furthest in from all
    /// its sides, roughly. `None` where there is no such polygon.
    fn middle(&self) -> Option<Vec3> {
        let Polygon::Cut(polygon) = &self.polygon else {
            return None;
        };
        let (mut area, mut x, mut y) = (0.0_f64, 0.0_f64, 0.0_f64);
        for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
            let cross = a[0] * b[1] - a[1] * b[0];
            area += cross;
            x += (a[0] + b[0]) * cross;
            y += (a[1] + b[1]) * cross;
        }
        if !(area.is_finite() && area.abs() > 1e-12 * self.reach * self.reach) {
            return None;
        }
        let (ex, ey) = self.axes;
        let middle = self.centre + ex * (x / (3.0 * area)) + ey * (y / (3.0 * area));
        (middle.is_finite() && self.sees(middle) == Seen::Maybe).then_some(middle)
    }
}

/// The least singular value of the carry's fit (below) that counts as the
/// sides narrowing, spreading or turning across the end: a millionth of
/// the end's size per unit of it.
const TAPER_LEAST: f64 = 1e-6;

/// The centre of the taper: the place on the end's plane that the sides,
/// carried on to the plane `length` beyond the end, do not move in it.
/// Where the sides narrow toward a point (a cone toward its apex, a loft
/// through sections drawn and made smaller about one line) it is the foot
/// of that point or line, and a tip over it continues the body's own axis;
/// the centre of area of what the sides come to (the shoulder) lies there
/// only where the end is symmetric about it.
///
/// The carry (each point of the rim to the point of the shoulder at the
/// same parameter, both read in the end's plane with `axes` about
/// `centre`) is fitted by an affine map, least squares along the rim's
/// length (Gauss–Legendre, 8 points on each quarter of every knot span):
/// `x ↦ x + J x + d`. Its fixed point solves `J f + d = 0`. Along a
/// direction in which `J` is nothing (within [`TAPER_LEAST`] and the
/// fit's own misfit: sides that run on parallel, as a prism's, or lean
/// without narrowing) the place is the shoulder's centre of area; along
/// one in which it is not, the fixed point's. `None` where `J` is nothing
/// both ways (a prism, a body that only leans: the place is then the
/// shoulder's centre as before), or the shoulder cannot be had.
fn taper_centre(
    rims: &[TipRim],
    centre: Vec3,
    normal: Vec3,
    axes: (Vec3, Vec3),
    length: f64,
    tol: f64,
) -> Option<Vec3> {
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    let (_, shoulder) = tip_shoulder(rims, centre + normal * length, normal, tol).ok()?;
    if shoulder.len() != rims.len() {
        return None;
    }
    let (ex, ey) = axes;
    let mut size = 0.0_f64;
    for rim in rims {
        for p in rim.curve.control_points() {
            size = size
                .max((*p - centre).dot(ex).abs())
                .max((*p - centre).dot(ey).abs());
        }
    }
    if !(size.is_finite() && size > 0.0) {
        return None;
    }
    // Flat coordinates in units of the end's size.
    let flat = |p: Vec3| [(p - centre).dot(ex) / size, (p - centre).dot(ey) / size];
    let rule = Rule::new(8);
    // The normal equations of the fit: per sample `w [x y 1]ᵀ [x y 1]` and
    // `w [x y 1]ᵀ Δ`, `Δ` the carry less the place.
    let mut normal_eq = [[0.0_f64; 3]; 3];
    let mut rhs = [[0.0_f64; 2]; 3];
    let mut samples: Vec<([f64; 3], [f64; 2], f64)> = Vec::new();
    for (rim, carried) in rims.iter().zip(&shoulder) {
        let curve = &rim.curve;
        if curve.domain() != carried.domain() {
            return None;
        }
        for pair in curve.knots().windows(2) {
            let [lo, hi] = pair else { continue };
            if hi <= lo {
                continue;
            }
            let quarter = 0.25 * (hi - lo);
            for panel in 0..4u8 {
                let from = lo + quarter * f64::from(panel);
                for (t, weight) in rule.points(from, from + quarter) {
                    let (point, along) = curve.derivatives(t);
                    let w = weight * along.length() / size;
                    let [x, y] = flat(point);
                    let [sx, sy] = flat(carried.eval(t));
                    let row = [x, y, 1.0];
                    let moved = [sx - x, sy - y];
                    for (i, &ri) in row.iter().enumerate() {
                        let (Some(eq), Some(r)) = (normal_eq.get_mut(i), rhs.get_mut(i)) else {
                            continue;
                        };
                        for (cell, &rj) in eq.iter_mut().zip(&row) {
                            *cell += w * ri * rj;
                        }
                        for (cell, &m) in r.iter_mut().zip(&moved) {
                            *cell += w * ri * m;
                        }
                    }
                    samples.push((row, moved, w));
                }
            }
        }
    }
    let [[a1, a2], [b1, b2], [c1, c2]] = solve3(normal_eq, rhs)?;
    // The carry's map: moved = J [x, y] + d.
    let j = [[a1, b1], [a2, b2]];
    let d = [c1, c2];
    // How far the carry is from that map (root mean square, in units of
    // the end's size).
    let (mut misfit, mut total) = (0.0_f64, 0.0_f64);
    for ([x, y, _], [mx, my], w) in &samples {
        let fx = j[0][0] * x + j[0][1] * y + d[0] - mx;
        let fy = j[1][0] * x + j[1][1] * y + d[1] - my;
        misfit += w * (fx * fx + fy * fy);
        total += w;
    }
    let misfit = if total > 0.0 {
        (misfit / total).sqrt()
    } else {
        return None;
    };
    // The singular values and vectors of J, from Jᵀ J.
    let p = j[0][0] * j[0][0] + j[1][0] * j[1][0];
    let q = j[0][0] * j[0][1] + j[1][0] * j[1][1];
    let r = j[0][1] * j[0][1] + j[1][1] * j[1][1];
    let half = 0.5 * (p + r);
    let spread = (0.25 * (p - r) * (p - r) + q * q).sqrt();
    let (big, small) = (half + spread, (half - spread).max(0.0));
    let first = if q.abs() > 0.0 {
        let (vx, vy) = (big - r, q);
        let norm = (vx * vx + vy * vy).sqrt();
        [vx / norm, vy / norm]
    } else if p >= r {
        [1.0, 0.0]
    } else {
        [0.0, 1.0]
    };
    let second = [-first[1], first[0]];
    let least = TAPER_LEAST.max(misfit);
    let shoulder_refs: Vec<&NurbsCurve> = shoulder.iter().collect();
    let (_, middle) = area_and_centre(&shoulder_refs, normal)?;
    let g = flat(middle);
    let mut place = [0.0_f64; 2];
    let mut tapered = false;
    for (v, sigma2) in [(first, big), (second, small)] {
        let sigma = sigma2.sqrt();
        let along = if sigma > least {
            tapered = true;
            // u = J v / σ; along v the fixed point is −(u · d) / σ.
            let u = [
                (j[0][0] * v[0] + j[0][1] * v[1]) / sigma,
                (j[1][0] * v[0] + j[1][1] * v[1]) / sigma,
            ];
            -(u[0] * d[0] + u[1] * d[1]) / sigma
        } else {
            v[0] * g[0] + v[1] * g[1]
        };
        place[0] += v[0] * along;
        place[1] += v[1] * along;
    }
    if !tapered {
        return None;
    }
    let found = centre + ex * (place[0] * size) + ey * (place[1] * size);
    found.is_finite().then_some(found)
}

/// `N X = R` for a 3 × 3 `N` and two right-hand sides, by Cramer's rule
/// (the normal equations of a fit in coordinates of the size of one about
/// the centre of area are well conditioned); `None` where `N` is singular
/// to rounding.
fn solve3(n: [[f64; 3]; 3], r: [[f64; 2]; 3]) -> Option<[[f64; 2]; 3]> {
    let det3 = |m: [[f64; 3]; 3]| {
        let [[a, b, c], [d, e, f], [g, h, i]] = m;
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    };
    let scale = n.iter().flatten().fold(0.0_f64, |m, x| m.max(x.abs()));
    let det = det3(n);
    if !(scale.is_finite() && det.is_finite() && det.abs() > 1e-12 * scale * scale * scale) {
        return None;
    }
    let [[a, b, c], [d, e, f], [g, h, i]] = n;
    let column = |k: [f64; 3]| {
        let [k0, k1, k2] = k;
        [
            det3([[k0, b, c], [k1, e, f], [k2, h, i]]) / det,
            det3([[a, k0, c], [d, k1, f], [g, k2, i]]) / det,
            det3([[a, b, k0], [d, e, k1], [g, h, k2]]) / det,
        ]
    };
    let [[r0, s0], [r1, s1], [r2, s2]] = r;
    let [x0, y0, z0] = column([r0, r1, r2]);
    let [x1, y1, z1] = column([s0, s1, s2]);
    let x = [[x0, x1], [y0, y1], [z0, z1]];
    x.iter().flatten().all(|v| v.is_finite()).then_some(x)
}

/// A refusal of the nets, with the edge of the rim curve it names, said
/// as what is true of the end where the point's place is to blame;
/// `longest` looks for the longest length that fits over the same place,
/// asked where a length given is too long.
fn tip_refusal(
    end: FaceId,
    error: TipError,
    edge_of: &dyn Fn(usize) -> Option<EdgeId>,
    seen: &dyn Fn() -> Seen,
    at_given: bool,
    longest: Option<&dyn Fn() -> Option<f64>>,
) -> KernelError {
    let sight = matches!(
        error,
        TipError::NotStarShaped { .. }
            | TipError::ClosesIn { .. }
            | TipError::NotProven { .. }
            | TipError::NoLengthFits
    );
    if sight {
        match seen() {
            Seen::Nowhere => {
                return refused(
                    vec![end],
                    None,
                    "no point over this end sees its whole outline (the end is not star-shaped, as a crescent or a cambered blade section is not), and a tip that closes in one point needs a point that does: such an end needs a tip that closes in a ridge, or a blend, which is not built yet",
                );
            }
            Seen::NotFrom if at_given => {
                return refused_as(
                    "at",
                    vec![end],
                    None,
                    "the place given for the tip does not see the whole outline of the end: give a place from which all of it is in view",
                );
            }
            Seen::NotFrom => {
                return refused(
                    vec![end],
                    None,
                    "the end's centre of area does not see the whole outline of the end (an end that is not convex can have it so), and no other place that does was found: give at, a place from which all of it is in view",
                );
            }
            Seen::Maybe => {}
        }
    }
    // A length given that is too long: the longest that fits over the
    // same place, where one is found.
    let too_long = matches!(
        error,
        TipError::NotStarShaped { .. } | TipError::ClosesIn { .. }
    );
    let fits = if too_long {
        longest.and_then(|find| find())
    } else {
        None
    };
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
        TipError::NotStarShaped { .. } | TipError::ClosesIn { .. } if fits.is_some() => {
            "; give a shorter length (no longer than the longest that fits, below), or a place for the tip further inside the end"
        }
        TipError::NotStarShaped { .. } | TipError::ClosesIn { .. } => {
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
    let blame = match &error {
        TipError::NotStarShaped { .. }
        | TipError::ClosesIn { .. }
        | TipError::NoLengthFits
        | TipError::RimReachesPlane { .. } => "length",
        TipError::NotProven { .. } if at_given => "at",
        _ => "face",
    };
    let said = match &error {
        // The search's own words say more than is known here.
        TipError::NoLengthFits => "no length tried fits (from twice the radius of the circle of the end's area, halved again and again down to four tolerances): at each the faces round the end close in before the point, or are seen from it edge-on; give a length, or a place for the point (at)".to_string(),
        _ => format!("{error}{advice}"),
    };
    KernelError::Tip {
        faces: vec![end],
        edge: rim.and_then(edge_of),
        blame,
        reason: said,
        longest: fits,
    }
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
    let mut found: Option<(TipRim, Border, bool)> = None;
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
        let (rim, against) = if first.distance(start) <= tol && last.distance(finish) <= tol {
            (rim, false)
        } else if first.distance(finish) <= tol && last.distance(start) <= tol {
            match rim.reversed() {
                Ok(rim) => (rim, true),
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
        found = Some((rim, border, against));
    }
    let (rim, border, against) = found.ok_or_else(|| {
        at_side(
            "the edge is not a whole border of the spline surface of the face behind it (that face was cut there, or its surface runs on past the end): a tip over such a side is not built yet".to_string(),
        )
    })?;
    Ok(RimEdge {
        coedge,
        side,
        rim,
        surface,
        border,
        against,
    })
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
