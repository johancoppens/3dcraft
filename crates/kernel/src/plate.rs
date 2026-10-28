//! Plates: faces of a body (a sheet, developable or not) given a
//! thickness on a side of them, as a solid of their own, and the joints
//! along their edges with neighbouring plates (docs/design/plates.md).
//!
//! **The solid.** A plate of the faces `F` at the levels `lo < hi` (along
//! each face's outward normal: `0..t` out, `−t..0` in, `−t/2..t/2` in the
//! middle) is the set of points `p + s N(p)` for `p` in `F` and `s` in
//! `[lo, hi]`. Its faces are the faces of `F` offset to `hi` (the front
//! skin, facing along `N`) and to `lo` (the back skin, facing against it),
//! and one *rim* per boundary edge of `F` (an edge only one face of `F`
//! uses). Edges inside `F` become edges between the skins of the two faces
//! at each level: the faces must be tangent there, within what a quarter
//! of the tolerance allows at the larger level, since a crease would open
//! the skins on one side and cross them on the other.
//!
//! **Rims and joints.** Along a boundary edge `C` the rim is the surface
//! the plate ends on. Every rim is found the same way: at stations along
//! `C`, in the plane `Π` through `C(t)` square to its tangent, the two
//! points where the rim meets the skins, each the meeting point of two
//! surfaces and `Π` (or of a skin and two planes), found by Newton's
//! method from the tangent planes' answer ([`crate::tweak::meet`]).
//!
//! - *Square*: the rim is the surface swept by the normal of the faces
//!   along `C`: its corners are `C + lo N` and `C + hi N`.
//! - *Bevel* by an angle `α`: the line through `C` in `Π` turned from `N`
//!   by `α` about the tangent, positive outward at the front.
//! - With a neighbouring plate (a face of another body that `C` runs along
//!   the boundary of, its thickness and side) the joint is decided by
//!   which skin of each plate faces the other plate's material (the sign of
//!   the other's inward direction along this one's normal):
//!   - *Mitre*: the rim runs from where the skins of both plates that face
//!     away from each other's material meet to where the skins that face
//!     it meet; the neighbour's mitre runs between the same two curves, so
//!     the two rims are one surface (on the bisector when the plates are
//!     equally thick).
//!   - *Touch*: square rims that meet where the skins facing each other's
//!     material meet: the inner edges of two stitch-and-glue panels just
//!     touch, the gap opens outward.
//!   - *Over*: this plate covers the neighbour's edge: its rim lies on the
//!     neighbour's skin that faces away from this plate's material (flush
//!     with it), between this plate's two skins.
//!   - *Under*: the neighbour covers this plate's edge: the rim lies on the
//!     neighbour's skin that faces this plate's material. One plate over
//!     and the other under share the line where the covering plate's near
//!     skin meets the covered plate's far skin: the deck rests on the
//!     sheer panel.
//!
//!   The rims of mitre, touch, square and bevel are ruled (a B-spline of
//!   degree 1 across, its rulings the segments between the two corners at
//!   each station, which is what the joint's rim is); the rims of over and
//!   under are pieces of the neighbour's skin itself, so the plates touch
//!   there exactly.
//!
//! **Corners.** At a corner of `F` (a vertex where two boundary edges
//! meet) the plate's vertex on each skin is where that skin and the two
//! rims meet, and the edge between them is where the two rims meet. The
//! rims and the edges of each boundary edge are traced past its ends by
//! enough to reach those corners (the edge's curve continued, its points
//! put on the faces' surface continued), and the reach is doubled while a
//! corner lies past it.
//!
//! **Curves and surfaces kept simple.** Each edge is the simplest of a
//! line, a circle and a cubic spline through its traced points that holds
//! them within a quarter of the tolerance; a ruled rim whose control
//! points lie in one plane within that is the plane. The traced points are
//! added where the spline through them leaves the true curve by more than
//! a quarter of the tolerance at the middles of its steps.
//!
//! **Never a wrong solid.** The result passes `craft3d_topo::check`; that
//! no rim crosses another one or a skin (a plate thicker than the faces
//! are narrow, a joint at a reflex corner that runs into the plate) is for
//! the caller to check on the whole body, as for every operation that
//! builds a body from offsets (regeneration's `not_through_itself`).

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::KernelError;
use crate::offset_face::{
    FOLDS, NO_FIT, NO_NORMAL, NoOffset, Reaches, TURN_TOO_FAR, continued, continued_each,
    offset_surface_each,
};
use crate::offset_normal::TURN_SHARE;
use crate::tweak::meet;
use craft3d_geom::refine::interpolate;
use craft3d_geom::{Circle, Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Edge, EdgeId, Face, FaceId, VertexId, check};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::FRAC_PI_2;

const OPERATION: &str = "plate";

/// The share of the tolerance a traced curve or a fitted rim may be off
/// the true one by.
const SHARE: f64 = 0.25;

/// Steps a boundary edge is traced in to start with.
const FIRST_STEPS: usize = 16;

/// Steps the reach past each end of an edge is traced in.
const REACH_STEPS: usize = 4;

/// Rounds of adding the middles of steps where a fit is off.
const ROUNDS: usize = 12;

/// Most points one traced curve may have.
pub const MAX_STATIONS: usize = 4096;

/// Most faces one plate takes.
pub const MAX_PLATE_FACES: usize = 4096;

/// How much further than the traced curves the surfaces are continued
/// past their borders: a surface is continued by about a length, by its
/// fastest speed across the border (`offset_face::continued`), so where
/// it is slower it reaches less far; the curves must not run off it.
const SURFACE_REACH: f64 = 3.0;

/// How far from its face, as a length along the surface, a skin is held
/// to the tolerance ([`Layered::new`]), in multiples of the furthest the
/// plate's corners lie past the ends of its edges, with twice the
/// thicknesses beyond: everything the plate is made of lies within the
/// furthest reach of its faces (a corner further out is refused), and a
/// length along a straight path in parameters may be longer than the
/// distance on the surface; the twofold margin is for that, and for the
/// steps of the solvers on the way to a corner. A margin, made a
/// guarantee by checking every vertex and edge of the skins ([`held_by`]).
const HELD_REACH: f64 = 2.0;

/// Where the material of a plate lies against its faces, along their
/// outward normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// On the side the outward normal points to: levels `0..t`.
    Out,
    /// Behind the faces: levels `−t..0`.
    In,
    /// Half on each side: levels `−t/2..t/2`.
    Mid,
}

impl Side {
    /// The levels `(lo, hi)` of the two skins along the outward normal.
    pub fn levels(self, thickness: f64) -> (f64, f64) {
        match self {
            Side::Out => (0.0, thickness),
            Side::In => (-thickness, 0.0),
            Side::Mid => (-0.5 * thickness, 0.5 * thickness),
        }
    }
}

/// The plate across an edge: a face of another body whose boundary the
/// edge runs along, with that plate's thickness and side.
#[derive(Clone, Copy, Debug)]
pub struct Neighbour<'a> {
    pub body: &'a Body,
    pub face: FaceId,
    pub thickness: f64,
    pub side: Side,
}

/// How a plate ends along one boundary edge of its faces.
#[derive(Clone, Copy, Debug)]
pub enum Joint<'a> {
    /// Along the faces' normal.
    Square,
    /// Turned from the normal by this angle (radians, in (−π/2, π/2))
    /// about the edge, outward at the front for a positive angle.
    Bevel(f64),
    /// On the surface between where both plates' outer skins meet and
    /// where their inner ones meet.
    Mitre(Neighbour<'a>),
    /// Square, through where the skins facing each other's material meet.
    Touch(Neighbour<'a>),
    /// Covering the neighbour's edge, flush with its far skin.
    Over(Neighbour<'a>),
    /// Covered by the neighbour, resting on its near skin.
    Under(Neighbour<'a>),
    /// Ending on this surface (a datum plane, a stem): the rim is a piece
    /// of it between the two skins. Two plates whose ends lie on one
    /// surface meet there in one corner.
    Onto(&'a Surface),
}

impl<'a> Joint<'a> {
    fn neighbour(&self) -> Option<Neighbour<'a>> {
        match self {
            Joint::Square | Joint::Bevel(_) | Joint::Onto(_) => None,
            Joint::Mitre(n) | Joint::Touch(n) | Joint::Over(n) | Joint::Under(n) => Some(*n),
        }
    }
}

/// What a face of a plate was made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    /// The front skin (at `hi`) of this face of the input.
    Front(FaceId),
    /// The back skin (at `lo`) of this face of the input.
    Back(FaceId),
    /// The rim along this boundary edge of the input.
    Rim(EdgeId),
}

/// A plate and what each of its faces was made from (`faces[i]` for face
/// `i` of `body`).
#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    pub body: Body,
    pub faces: Vec<Made>,
}

fn refuse(edge: Option<EdgeId>, reason: impl Into<String>) -> KernelError {
    KernelError::Plate {
        edge,
        reason: reason.into(),
    }
}

fn at(p: Vec3) -> String {
    format!("({}, {}, {})", short(p.x), short(p.y), short(p.z))
}

fn short(v: f64) -> String {
    if v != 0.0 && v.abs() < 1e-3 {
        return format!("{v:.3e}");
    }
    let rounded = format!("{v:.6}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn no_offset(why: NoOffset) -> &'static str {
    match why {
        NoOffset::Gone => {
            "the offset of the face vanishes: the thickness reaches a radius of the face on the side it goes to"
        }
        NoOffset::Folds => FOLDS,
        NoOffset::NoNormal => NO_NORMAL,
        NoOffset::NoFit => NO_FIT,
        NoOffset::Refused(why) => why,
    }
}

/// A face with its surface continued past its borders and its two skins.
struct Layered<'a> {
    face: &'a Face,
    /// The face's surface, continued.
    reference: Surface,
    /// +1, or −1 when the face's outward normal is against its surface's.
    sign: f64,
    /// The skins at `lo` and at `hi`.
    skins: [Surface; 2],
    levels: [f64; 2],
    /// How far from the face, as a length along its surface, its skins
    /// are held to the tolerance ([`Layered::new`]); infinite for a probe.
    held: f64,
}

impl<'a> Layered<'a> {
    /// The skins of `face` at `levels`, its surface continued past each
    /// border by `reaches` (a box of parameters) and its offsets held to
    /// the tolerance within the length `held` of its own domain, which is
    /// as far as anything of the plate lies from it.
    fn new(
        face: &'a Face,
        levels: (f64, f64),
        reaches: Reaches,
        held: f64,
        tolerance: &Tolerance,
    ) -> Result<Self, &'static str> {
        let reference = continued_each(&face.surface, reaches, tolerance);
        let skin = |level: f64| -> Result<Surface, &'static str> {
            if level == 0.0 {
                Ok(reference.clone())
            } else {
                offset_surface_each(face, level, reaches, None, Some(held), tolerance)
                    .map_err(no_offset)
            }
        };
        Ok(Self {
            face,
            sign: if face.reversed { -1.0 } else { 1.0 },
            skins: [skin(levels.0)?, skin(levels.1)?],
            reference,
            levels: [levels.0, levels.1],
            held,
        })
    }

    /// The face with its own surface only, for what is worked out before
    /// the skins are made (how far the joints reach, creases): its skins
    /// stand in as the surface itself and are not used.
    fn probe(face: &'a Face, levels: (f64, f64)) -> Self {
        Self {
            face,
            sign: if face.reversed { -1.0 } else { 1.0 },
            skins: [face.surface.clone(), face.surface.clone()],
            reference: face.surface.clone(),
            levels: [levels.0, levels.1],
            held: f64::INFINITY,
        }
    }

    /// The nearest point of the face's surface to `p` and the outward
    /// normal there.
    fn foot(&self, p: Vec3) -> Option<(Vec3, Vec3)> {
        let (u, v) = self.reference.project(p).ok()?;
        let normal = self.reference.normal(u, v)? * self.sign;
        let foot = self.reference.eval(u, v);
        (foot.is_finite() && normal.is_finite()).then_some((foot, normal))
    }
}

/// Where an edge is at one parameter, seen from the face it bounds.
#[derive(Clone, Copy, Debug)]
struct Station {
    /// The point of the edge, on the face's surface.
    point: Vec3,
    /// The unit tangent in the direction the face's loop runs.
    tangent: Vec3,
    /// The face's outward unit normal.
    normal: Vec3,
    /// Into the face, square to the edge: `normal × tangent`.
    inward: Vec3,
}

impl Station {
    /// The plane through the point square to the edge.
    fn plane(&self) -> Option<Surface> {
        Frame::from_z(self.point, self.tangent).map(|f| Surface::Plane(Plane::new(f)))
    }
}

fn station(edge: &Edge, t: f64, along: f64, face: &Layered<'_>) -> Option<Station> {
    station_on(&edge.curve, t, along, face)
}

/// [`station`] on the edge's curve `curve` (the edge's own, or that curve
/// continued past its ends, [`continued_past`]).
fn station_on(curve: &Curve3, t: f64, along: f64, face: &Layered<'_>) -> Option<Station> {
    let (p, d) = curve.derivatives(t);
    let (point, normal) = face.foot(p)?;
    let d = d * along;
    let tangent = (d - normal * d.dot(normal)).normalized()?;
    let inward = normal.cross(tangent).normalized()?;
    Some(Station {
        point,
        tangent,
        normal,
        inward,
    })
}

/// A neighbouring plate made ready: its face layered at its own levels,
/// and the coedges of that face's loops.
struct Near<'a> {
    body: &'a Body,
    layered: Layered<'a>,
    coedges: Vec<Coedge>,
}

/// The neighbour's outward normal and inward direction where it is
/// nearest `p`, and how far `p` is from its surface and from its boundary.
fn near_frame(near: &Near<'_>, p: Vec3) -> Option<(Vec3, Vec3, f64, f64)> {
    let (foot, normal) = near.layered.foot(p)?;
    let mut best: Option<(f64, Vec3)> = None;
    for coedge in &near.coedges {
        let Some(edge) = near.body.edge(coedge.edge) else {
            continue;
        };
        let Ok(raw) = edge.curve.project(p) else {
            continue;
        };
        let s = match edge.curve.period() {
            Some(period) if raw < edge.range.0 => {
                // Into the edge's range by whole turns.
                raw + ((edge.range.0 - raw) / period).ceil() * period
            }
            Some(_) => raw,
            None => raw,
        };
        let clamped = s.clamp(edge.range.0, edge.range.1);
        let distance = edge.curve.eval(clamped).distance(p);
        let (_, d) = edge.curve.derivatives(s);
        let d = if coedge.reversed { -d } else { d };
        if best.is_none_or(|(b, _)| distance < b) {
            best = Some((distance, d));
        }
    }
    let (off_edge, d) = best?;
    let tangent = (d - normal * d.dot(normal)).normalized()?;
    let inward = normal.cross(tangent).normalized()?;
    Some((normal, inward, foot.distance(p), off_edge))
}

/// What one boundary edge of the faces knows: its face, which way the
/// face's loop runs along it, and its joint.
struct Boundary<'a, 'j> {
    edge_id: EdgeId,
    edge: &'a Edge,
    face: usize,
    /// +1 when the loop runs along the edge's own direction.
    along: f64,
    joint: Joint<'j>,
    near: Option<Near<'j>>,
    /// The surface an `onto` end lies on, continued past its borders as
    /// the skins are (`None` until the reach is known, and for the other
    /// joints).
    end: Option<Surface>,
}

impl Boundary<'_, '_> {
    /// The surface an `onto` end lies on: continued once the reach is
    /// known, the given one before.
    fn end_surface(&self) -> Option<&Surface> {
        match self.joint {
            Joint::Onto(given) => Some(self.end.as_ref().unwrap_or(given)),
            _ => None,
        }
    }
}

/// The traced corners of a boundary edge: per parameter, the points on
/// the back and the front skin.
struct Traced {
    rows: [Vec<Vec3>; 2],
    fitted: [NurbsCurve; 2],
    /// The parameter range of the edge itself (without the reach).
    own: (f64, f64),
}

pub(crate) fn fit(params: &[f64], points: &[Vec3]) -> Result<NurbsCurve, KernelError> {
    let degree = params.len().saturating_sub(1).clamp(1, 3);
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(params, degree, &values)?;
    let ctrl: Vec<Vec3> = ctrl
        .iter()
        .map(|c| {
            Vec3::new(
                c.first().copied().unwrap_or(f64::NAN),
                c.get(1).copied().unwrap_or(f64::NAN),
                c.get(2).copied().unwrap_or(f64::NAN),
            )
        })
        .collect();
    Ok(NurbsCurve::polynomial(degree, knots, ctrl)?)
}

/// Points of `count` curves traced at the parameters `params` (sorted),
/// with the middles of steps added where the spline through each is
/// further than the share of the tolerance from the point there, until
/// none is.
pub(crate) fn trace(
    mut params: Vec<f64>,
    count: usize,
    edge: Option<EdgeId>,
    what: &str,
    at_param: &mut dyn FnMut(f64) -> Result<Vec<Vec3>, KernelError>,
    tol: f64,
) -> Result<(Vec<f64>, Vec<Vec<Vec3>>), KernelError> {
    let mut rows: Vec<Vec<Vec3>> = vec![Vec::with_capacity(params.len()); count];
    for &t in &params {
        let points = at_param(t)?;
        for (row, p) in rows.iter_mut().zip(points) {
            row.push(p);
        }
    }
    // The worst misfit of the last round: how far, where, at which point.
    let mut worst = (0.0_f64, 0.0_f64, Vec3::ZERO);
    let mut middles: BTreeMap<u64, Vec<Vec3>> = BTreeMap::new();
    for _ in 0..ROUNDS {
        let fitted: Vec<NurbsCurve> = rows
            .iter()
            .map(|row| fit(&params, row))
            .collect::<Result<_, _>>()?;
        let mut added: Vec<(usize, f64, Vec<Vec3>)> = Vec::new();
        worst = (0.0, 0.0, Vec3::ZERO);
        for (k, pair) in params.windows(2).enumerate() {
            let [a, b] = pair else { continue };
            let middle = 0.5 * (a + b);
            // A step that stayed whole since the last round has the same
            // middle: its points are kept, not traced again.
            let points = match middles.remove(&middle.to_bits()) {
                Some(points) => points,
                None => at_param(middle)?,
            };
            let mut off = false;
            for (curve, p) in fitted.iter().zip(&points) {
                let misfit = curve.eval(middle).distance(*p);
                if !(misfit <= SHARE * tol) {
                    off = true;
                    if !(misfit <= worst.0) {
                        worst = (misfit, middle, *p);
                    }
                }
            }
            if off {
                added.push((k + 1, middle, points));
            } else {
                middles.insert(middle.to_bits(), points);
            }
        }
        if added.is_empty() {
            return Ok((params, rows));
        }
        if params.len().saturating_add(added.len()) > MAX_STATIONS {
            break;
        }
        for (index, t, points) in added.into_iter().rev() {
            params.insert(index, t);
            for (row, p) in rows.iter_mut().zip(points) {
                row.insert(index, p);
            }
        }
    }
    let (misfit, t, p) = worst;
    Err(refuse(
        edge,
        format!(
            "the {what} could not be traced within a quarter of the tolerance in {MAX_STATIONS} points: the spline through them is still {} off at {} (parameter {})",
            short(misfit),
            at(p),
            short(t)
        ),
    ))
}

/// The simplest curve through `points` (a line, a circle, else the
/// spline): one that holds every point within the share of the tolerance.
fn simplest(points: &[Vec3], spline: &NurbsCurve, tol: f64) -> Curve3 {
    let limit = SHARE * tol;
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Curve3::Nurbs(spline.clone());
    };
    if first.distance(last) > tol
        && let Ok(line) = Line::through(first, last)
    {
        let off = |p: &Vec3| {
            let d = *p - first;
            (d - line.direction() * d.dot(line.direction())).length()
        };
        if points.iter().all(|p| off(p) <= limit) {
            return Curve3::Line(line);
        }
    }
    let middle = points.get(points.len() / 2).copied().unwrap_or(first);
    let closed = first.distance(last) <= tol;
    // A closed run: its middle and a quarter along, with the start.
    let third = if closed {
        points.get(points.len() / 4).copied().unwrap_or(first)
    } else {
        last
    };
    if let Some(circle) =
        circle_through(first, third, middle, closed).and_then(|circle| running_with(circle, points))
    {
        let off = |p: &Vec3| {
            let (x, y, z) = circle.frame().local(*p);
            (x.hypot(y) - circle.radius()).hypot(z)
        };
        if points.iter().all(|p| off(p) <= limit) {
            return Curve3::Circle(circle);
        }
    }
    Curve3::Nurbs(spline.clone())
}

/// The circle through three points, its parameter running from `a`
/// through `b` (for a closed run `b` is a quarter along and `c` half) in
/// the direction of the run.
fn circle_through(a: Vec3, b: Vec3, c: Vec3, closed: bool) -> Option<Circle> {
    let (ab, ac) = (b - a, c - a);
    let normal = ab.cross(ac);
    let n2 = normal.length_squared();
    if !(n2 > 0.0) {
        return None;
    }
    // Circumcentre of the triangle.
    let center = a
        + (normal.cross(ab) * ac.length_squared() + ac.cross(normal) * ab.length_squared())
            * (1.0 / (2.0 * n2));
    let radius = center.distance(a);
    // Runs a → b → c counter-clockwise about the normal when the points
    // are in that order on the circle; for an open run the order is a, c
    // (the middle), b (the last), so the normal of a, b, c is reversed.
    let z = if closed { normal } else { -normal };
    let frame = Frame::new(center, z, a - center)?;
    Circle::new(frame, radius).ok()
}

/// `circle` turned, if need be, to run the way `points` (on it, in order)
/// go round it: by the sign of the angle they turn through about its axis,
/// step by step. Three points of a run say which way it goes only while it
/// is less than a turn: a rim traced past both ends of an arc by more than
/// what is left of the circle (a slot whose end radius is about the
/// plate's thickness, the reach being twice the thickness) runs more than
/// a turn, and its first, middle and last points then lie the other way
/// round (the sheet-metal enclosure's and the rack's finding, 2026-10-27:
/// the edges went round the wrong half of their circles). `None` when the
/// points turn through no angle.
fn running_with(circle: Circle, points: &[Vec3]) -> Option<Circle> {
    let frame = *circle.frame();
    let (center, z) = (frame.origin(), frame.z());
    let turned: f64 = points
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => {
                let (a, b) = (*a - center, *b - center);
                Some(a.cross(b).dot(z).atan2(a.dot(b)))
            }
            _ => None,
        })
        .sum();
    if !(turned.abs() > 0.0) {
        return None;
    }
    if turned > 0.0 {
        return Some(circle);
    }
    let flipped = Frame::new(center, -z, frame.x())?;
    Circle::new(flipped, circle.radius()).ok()
}

/// The range of `curve` from `start` to `end` (two points on it), or
/// the whole of it for a closed run.
fn range_between(
    curve: &Curve3,
    start: Vec3,
    end: Vec3,
    closed: bool,
    tol: f64,
) -> Option<(f64, f64)> {
    let a = curve.project(start).ok()?;
    if !(curve.eval(a).distance(start) <= tol) {
        return None;
    }
    if closed {
        return match curve {
            Curve3::Nurbs(n) => Some(n.domain()),
            _ => curve.period().map(|p| (a, a + p)),
        };
    }
    let mut b = curve.project(end).ok()?;
    if !(curve.eval(b).distance(end) <= tol) {
        return None;
    }
    if let Some(period) = curve.period()
        && b <= a
    {
        b += period;
    }
    if let Curve3::Nurbs(n) = curve {
        // A corner at the very end of the traced run is past its reach.
        let (d0, d1) = n.domain();
        let room = 1e-9 * (d1 - d0).abs();
        if !(a > d0 + room || n.eval(d0).distance(start) <= SHARE * tol)
            || !(b < d1 - room || n.eval(d1).distance(end) <= SHARE * tol)
        {
            return None;
        }
    }
    (a < b).then_some((a, b))
}

/// The surface ruled between two splines on the same knots: `u` along
/// them, `v` from the first (0) to the second (1); the plane it lies in
/// when its control points lie in one within the share of the tolerance.
fn ruled(first: &NurbsCurve, second: &NurbsCurve, tol: f64) -> Result<Surface, KernelError> {
    if first.knots() != second.knots() || first.degree() != second.degree() {
        return Err(refuse(
            None,
            "the two sides of a rim were traced apart (a kernel bug)",
        ));
    }
    let count = first.control_points().len();
    let mut ctrl = Vec::with_capacity(2 * count);
    for (a, b) in first.control_points().iter().zip(second.control_points()) {
        ctrl.push(*a);
        ctrl.push(*b);
    }
    let surface = NurbsSurface::new(
        first.degree(),
        1,
        first.knots().to_vec(),
        vec![0.0, 0.0, 1.0, 1.0],
        count,
        2,
        ctrl.clone(),
        vec![1.0; 2 * count],
    )?;
    Ok(flat(&ctrl, tol).unwrap_or(Surface::Nurbs(surface)))
}

/// The surface of a ruled rim between the traced rows `first` and
/// `second` (the corners at each station, in order) and their splines:
/// [`ruled`] (the plane its control points lie in, else the ruled
/// spline), unless both rows are circles about one axis (their centres on
/// it within the share of the tolerance, apart along it by more than the
/// tolerance) and every traced corner and the middle of every ruling
/// between them lie within the share of the tolerance of the cylinder or
/// cone through the two circles: then that surface (a square rim round a
/// hole in a flat plate is the cylinder of the hole, exactly as an
/// extrusion's side is; a bevelled one a cone). The same share every
/// other choice of representation in this module holds to; a rim that is
/// one of these surfaces only roughly stays the spline.
fn ruled_rim(rows: [&[Vec3]; 2], fits: [&NurbsCurve; 2], tol: f64) -> Result<Surface, KernelError> {
    let surface = ruled(fits[0], fits[1], tol)?;
    if matches!(surface, Surface::Plane(_)) {
        return Ok(surface);
    }
    Ok(round_rim(rows, fits, tol).unwrap_or(surface))
}

/// The cylinder or cone of [`ruled_rim`], if the rows make one.
fn round_rim(rows: [&[Vec3]; 2], fits: [&NurbsCurve; 2], tol: f64) -> Option<Surface> {
    let limit = SHARE * tol;
    let (Curve3::Circle(c0), Curve3::Circle(c1)) = (
        simplest(rows[0], fits[0], tol),
        simplest(rows[1], fits[1], tol),
    ) else {
        return None;
    };
    let (o0, o1) = (c0.frame().origin(), c1.frame().origin());
    let mut axis = c0.frame().z();
    let mut height = (o1 - o0).dot(axis);
    if height < 0.0 {
        axis = -axis;
        height = -height;
    }
    // Coaxial: the second centre on the first's axis, the second circle's
    // plane square to it (its axis along it within what the radius allows).
    let off_axis = ((o1 - o0) - axis * height).length();
    let tilt = c1.frame().z().cross(axis).length() * c1.radius();
    if !(height > tol && off_axis <= limit && tilt <= limit) {
        return None;
    }
    let frame = Frame::new(o0, axis, c0.frame().x())?;
    let (r0, r1) = (c0.radius(), c1.radius());
    let surface = if (r1 - r0).abs() <= limit {
        Surface::Cylinder(craft3d_geom::Cylinder::new(frame, 0.5 * (r0 + r1)).ok()?)
    } else {
        let half = ((r1 - r0) / height).atan();
        Surface::Cone(craft3d_geom::Cone::new(frame, r0, half).ok()?)
    };
    // The distance of a point from it, in the frame of the axis.
    let (radius, slope) = match &surface {
        Surface::Cylinder(c) => (c.radius(), 0.0),
        Surface::Cone(c) => (c.radius(), c.half_angle().tan()),
        _ => return None,
    };
    let off = |p: Vec3| {
        let (x, y, z) = frame.local(p);
        ((x.hypot(y) - (radius + z * slope)) / slope.hypot(1.0)).abs()
    };
    let holds = rows[0]
        .iter()
        .zip(rows[1])
        .all(|(a, b)| off(*a) <= limit && off(*b) <= limit && off(a.lerp(*b, 0.5)) <= limit);
    (holds && rows[0].len() == rows[1].len()).then_some(surface)
}

/// The plane `points` lie in within the share of the tolerance, if any.
fn flat(points: &[Vec3], tol: f64) -> Option<Surface> {
    let first = *points.first()?;
    let far = points
        .iter()
        .copied()
        .max_by(|a, b| a.distance(first).total_cmp(&b.distance(first)))?;
    let along = far - first;
    let third = points.iter().copied().max_by(|a, b| {
        along
            .cross(*a - first)
            .length()
            .total_cmp(&along.cross(*b - first).length())
    })?;
    let normal = along.cross(third - first).normalized()?;
    if points
        .iter()
        .all(|p| (*p - first).dot(normal).abs() <= SHARE * tol)
    {
        Frame::new(first, normal, along).map(|f| Surface::Plane(Plane::new(f)))
    } else {
        None
    }
}

const PAST_REACH: &str =
    "the joint reaches past the faces' surfaces continued beyond their borders";

/// The most a border's reach is raised by (a vertex where the surface
/// moves across the border at a 64th of its fastest there): beyond it the
/// continuation would be extrapolated far past anything the surface says,
/// and a corner it still does not reach is refused (`PAST_REACH`).
const MOST_REACH_FACTOR: f64 = 64.0;

/// How far to continue a face's freeform surface for it to reach `reach`
/// past every vertex of the face that lies on a border of its parameters.
/// A surface is continued past a border by the parameter that takes its
/// fastest point there `reach` far (`offset_face::continued`), so a point
/// of the border where it is slower reaches less: on a developable's face
/// across its rulings, as far as each ruling is long, so a corner by a
/// short ruling beside long ones fell short (the sea kayak's panel,
/// 2026-10-27: a ruling of 14.5 mm, the longest 190 mm). The corners of a
/// plate lie past the ends of its edges, at the face's vertices, so the
/// reach is raised by the most any of them needs: the border's fastest
/// speed over the speed at the vertex, at most [`MOST_REACH_FACTOR`]. A
/// vertex where the surface moves across the border by no more than the
/// tolerance over that parameter's range (a collapsed corner, a fan's
/// apex) needs nothing there.
fn reach_past_vertices(body: &Body, face: &Face, reach: f64, tol: f64) -> Reaches {
    let nurbs = match &face.surface {
        Surface::Nurbs(n) => std::borrow::Cow::Borrowed(n),
        Surface::Bezier(b) => match NurbsSurface::from_bezier(b) {
            Ok(n) => std::borrow::Cow::Owned(n),
            Err(_) => return [reach; 4],
        },
        _ => return [reach; 4],
    };
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    // The fastest speed across a border, as `offset_face::reached` finds it.
    let fastest = |along_u: bool, at: f64| {
        (0..=8)
            .map(|k| {
                let f = f64::from(k) / 8.0;
                let (_, du, dv) = if along_u {
                    nurbs.derivatives(at, v0 + (v1 - v0) * f)
                } else {
                    nurbs.derivatives(u0 + (u1 - u0) * f, at)
                };
                if along_u { du.length() } else { dv.length() }
            })
            .fold(0.0_f64, f64::max)
    };
    let mut factor: [f64; 4] = [1.0; 4];
    for &lp in &face.loops {
        let Some(l) = body.loop_(lp) else {
            continue;
        };
        for coedge in &l.coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            for vertex in [edge.start, edge.end] {
                let Some(point) = body.vertex(vertex).map(|v| v.point) else {
                    continue;
                };
                let Ok((u, v)) = face.surface.project(point) else {
                    continue;
                };
                let (_, su, sv) = nurbs.derivatives(u, v);
                // A vertex within the reach of a border (on it, or as near
                // as the face's fit leaves it) needs that border continued.
                let (speed_u, speed_v) = (su.length(), sv.length());
                for (border, (on, along_u, at, speed)) in [
                    ((u - u0) * speed_u <= reach, true, u0, speed_u),
                    ((u1 - u) * speed_u <= reach, true, u1, speed_u),
                    ((v - v0) * speed_v <= reach, false, v0, speed_v),
                    ((v1 - v) * speed_v <= reach, false, v1, speed_v),
                ]
                .into_iter()
                .enumerate()
                {
                    // A vertex where the surface moves across the border
                    // by no more than the tolerance over the whole range
                    // of that parameter is where it collapses (a fan's
                    // apex): nothing is reached past it.
                    let range = if along_u { u1 - u0 } else { v1 - v0 };
                    if on && speed * range.abs() > tol && speed.is_finite() {
                        let ratio = (fastest(along_u, at) / speed).min(MOST_REACH_FACTOR);
                        if let Some(f) = factor.get_mut(border)
                            && ratio.is_finite()
                        {
                            *f = f.max(ratio);
                        }
                    }
                }
            }
        }
    }
    factor.map(|f| reach * f)
}

/// True when the nearest point of the freeform `surface` to `p` is on the
/// border of its parameters, in a direction it does not close in.
fn at_border(surface: &Surface, p: Vec3, tol: f64) -> bool {
    let nurbs = match surface {
        Surface::Nurbs(n) => std::borrow::Cow::Borrowed(n),
        Surface::Bezier(b) => match NurbsSurface::from_bezier(b) {
            Ok(n) => std::borrow::Cow::Owned(n),
            Err(_) => return false,
        },
        _ => return false,
    };
    let Ok((u, v)) = surface.project(p) else {
        return false;
    };
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    let on = |x: f64, lo: f64, hi: f64| {
        let room = 1e-9 * (hi - lo).abs();
        x <= lo + room || x >= hi - room
    };
    (on(u, u0, u1) && !nurbs.closes(true, tol)) || (on(v, v0, v1) && !nurbs.closes(false, tol))
}

/// The outward normal of `surface` near `p`, as its surface normal.
fn surface_normal(surface: &Surface, p: Vec3) -> Option<Vec3> {
    let (u, v) = surface.project(p).ok()?;
    surface.normal(u, v)
}

/// A plate of `faces` of `body` (see the module documentation): levels
/// from `side` and `thickness`, and per boundary edge of the faces its
/// joint (square where none is given).
///
/// **Errors.** [`KernelError::Plate`] with the edge it is about and why:
/// faces that do not hang together, a crease between two of them, a
/// joint whose neighbour the edge does not run along, plates that meet in
/// line, a corner past the reach, a curve that cannot be traced;
/// [`KernelError::InvalidSize`] for a thickness that is not more than the
/// tolerance; [`KernelError::InvalidBody`] when the result fails the
/// checker.
pub fn plate(
    body: &Body,
    faces: &[FaceId],
    thickness: f64,
    side: Side,
    joints: &BTreeMap<EdgeId, Joint<'_>>,
    tolerance: &Tolerance,
) -> Result<Plate, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if !(thickness.is_finite() && thickness > tol) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: thickness,
        });
    }
    if faces.is_empty() {
        return Err(refuse(None, "a plate needs at least one face"));
    }
    if faces.len() > MAX_PLATE_FACES {
        return Err(refuse(
            None,
            format!("a plate takes at most {MAX_PLATE_FACES} faces"),
        ));
    }
    let mut chosen: Vec<FaceId> = Vec::with_capacity(faces.len());
    for face in faces {
        if body.face(*face).is_none() {
            return Err(refuse(None, format!("the body has no face {}", face.0)));
        }
        if !chosen.contains(face) {
            chosen.push(*face);
        }
    }
    for joint in joints.values() {
        if let Joint::Bevel(angle) = joint
            && !(angle.abs() < FRAC_PI_2)
        {
            return Err(refuse(
                None,
                format!(
                    "a bevel's angle is between −90 and 90 degrees, got {}",
                    angle.to_degrees()
                ),
            ));
        }
        if let Some(n) = joint.neighbour()
            && !(n.thickness.is_finite() && n.thickness > tol)
        {
            return Err(KernelError::InvalidSize {
                name: "neighbour thickness",
                value: n.thickness,
            });
        }
    }
    build(
        body, &chosen, thickness, side, joints, HELD_REACH, tolerance,
    )
}

/// How the faces use an edge.
#[derive(Clone, Debug)]
enum Use {
    /// One face of the plate: (index into the faces, loop runs along it).
    Boundary(usize, bool),
    /// Two uses (two faces, or one face twice along a seam).
    Inner([(usize, bool); 2]),
}

#[allow(clippy::too_many_arguments)]
fn build(
    body: &Body,
    chosen: &[FaceId],
    thickness: f64,
    side: Side,
    joints: &BTreeMap<EdgeId, Joint<'_>>,
    held_reach: f64,
    tolerance: &Tolerance,
) -> Result<Plate, KernelError> {
    let tol = tolerance.linear;
    let levels = side.levels(thickness);
    // The faces, first with their own surfaces only: the skins are made
    // once it is known how far the joints reach past the edges.
    let mut layered: Vec<Layered<'_>> = Vec::with_capacity(chosen.len());
    for id in chosen {
        let face = body
            .face(*id)
            .ok_or_else(|| refuse(None, "a face of the plate is missing"))?;
        layered.push(Layered::probe(face, levels));
    }
    // How each edge is used.
    let mut uses: BTreeMap<EdgeId, Vec<(usize, bool)>> = BTreeMap::new();
    for (index, made) in layered.iter().enumerate() {
        for loop_ in made.face.loops.iter().filter_map(|l| body.loop_(*l)) {
            for coedge in &loop_.coedges {
                uses.entry(coedge.edge)
                    .or_default()
                    .push((index, !coedge.reversed));
            }
        }
    }
    let mut kinds: BTreeMap<EdgeId, Use> = BTreeMap::new();
    for (edge, list) in &uses {
        let kind = match list.as_slice() {
            [one] => Use::Boundary(one.0, one.1),
            [a, b] if a.1 != b.1 => Use::Inner([*a, *b]),
            _ => {
                return Err(refuse(
                    Some(*edge),
                    "an edge used more than twice, or twice the same way, by the faces: they do not make one sheet",
                ));
            }
        };
        kinds.insert(*edge, kind);
    }
    for edge in joints.keys() {
        if !matches!(kinds.get(edge), Some(Use::Boundary(..))) {
            return Err(refuse(
                Some(*edge),
                "a joint is given for an edge that is not on the boundary of the plate's faces",
            ));
        }
    }
    connected(&layered, &kinds)?;
    // A crease is refused before anything is traced: the skins of the two
    // faces would not meet at the corners either.
    for (edge_id, kind) in &kinds {
        let Use::Inner([a, b]) = kind else { continue };
        let (Some(edge), Some(fa), Some(fb)) =
            (body.edge(*edge_id), layered.get(a.0), layered.get(b.0))
        else {
            continue;
        };
        for k in 0..=CREASE_SAMPLES {
            let t =
                edge.range.0 + (edge.range.1 - edge.range.0) * (k as f64 / CREASE_SAMPLES as f64);
            creased(*edge_id, edge.curve.eval(t), fa, fb, levels, tol)?;
        }
    }
    // The boundary edges with their joints and neighbours.
    let mut boundaries: Vec<Boundary<'_, '_>> = Vec::new();
    for (edge_id, kind) in &kinds {
        let Use::Boundary(face, along) = kind else {
            continue;
        };
        let edge = body
            .edge(*edge_id)
            .ok_or_else(|| refuse(Some(*edge_id), "the edge is missing"))?;
        let joint = joints.get(edge_id).copied().unwrap_or(Joint::Square);
        let near = match joint.neighbour() {
            None => None,
            Some(n) => {
                let face = n
                    .body
                    .face(n.face)
                    .ok_or_else(|| refuse(Some(*edge_id), "the neighbour has no such face"))?;
                let layered = Layered::probe(face, n.side.levels(n.thickness));
                let coedges = face
                    .loops
                    .iter()
                    .filter_map(|l| n.body.loop_(*l))
                    .flat_map(|l| l.coedges.iter().copied())
                    .collect();
                Some(Near {
                    body: n.body,
                    layered,
                    coedges,
                })
            }
        };
        boundaries.push(Boundary {
            edge_id: *edge_id,
            edge,
            face: *face,
            along: if *along { 1.0 } else { -1.0 },
            joint,
            near,
            end: None,
        });
    }
    // How far each boundary edge is traced past its ends: to where the
    // plate's corners lie by the tangent planes, and a margin beyond.
    // Surfaces ends lie on, continued for the estimate as far as the
    // margin of the reach goes (and further below, once it is known).
    let first_reach = SURFACE_REACH
        * 2.0
        * (thickness
            + boundaries
                .iter()
                .filter_map(|b| b.joint.neighbour().map(|n| n.thickness))
                .fold(0.0_f64, f64::max));
    for boundary in &mut boundaries {
        if let Joint::Onto(given) = boundary.joint {
            let reach = first_reach + span(boundary.edge);
            boundary.end = Some(continued(given, reach, [true; 4], tolerance));
        }
    }
    let reaches = reaches(&boundaries, &layered, thickness, tolerance)?;
    let furthest = reaches
        .values()
        .flat_map(|(a, b)| [*a, *b])
        .fold(0.0_f64, f64::max);
    let surface_reach = SURFACE_REACH * furthest + thickness;
    let held = held_reach * (furthest + thickness);
    for (made, id) in layered.iter_mut().zip(chosen) {
        let reach = reach_past_vertices(body, made.face, surface_reach, tolerance.linear);
        *made = Layered::new(made.face, levels, reach, held, tolerance)
            .map_err(|why| refuse(None, format!("face {}: {why}", id.0)))?;
    }
    for boundary in &mut boundaries {
        // A surface an end lies on is continued as the skins are: the
        // plate's end may run past the face it was given as (a stem).
        if let Joint::Onto(given) = boundary.joint {
            let reach = surface_reach + span(boundary.edge);
            boundary.end = Some(continued(given, reach, [true; 4], tolerance));
        }
        if let (Some(near), Some(n)) = (boundary.near.as_mut(), boundary.joint.neighbour()) {
            near.layered = Layered::new(
                near.layered.face,
                n.side.levels(n.thickness),
                reach_past_vertices(
                    near.body,
                    near.layered.face,
                    surface_reach + n.thickness,
                    tolerance.linear,
                ),
                held + held_reach * n.thickness,
                tolerance,
            )
            .map_err(|why| {
                refuse(
                    Some(boundary.edge_id),
                    format!("the neighbour's face: {why}"),
                )
            })?;
        }
    }
    // Trace the corners of every boundary edge, and make its rim.
    let mut traced: BTreeMap<EdgeId, Traced> = BTreeMap::new();
    let mut rims: BTreeMap<EdgeId, (Surface, bool)> = BTreeMap::new();
    for boundary in &boundaries {
        let face = layered
            .get(boundary.face)
            .ok_or_else(|| refuse(None, "a face of the plate is missing"))?;
        let reach = reaches
            .get(&boundary.edge_id)
            .copied()
            .unwrap_or((0.0, 0.0));
        let (made, rim) = trace_boundary(boundary, face, reach, tolerance)?;
        rims.insert(boundary.edge_id, rim);
        traced.insert(boundary.edge_id, made);
    }
    // Vertices of the plate: per vertex of the faces and level.
    let mut vertex_faces: BTreeMap<VertexId, BTreeSet<usize>> = BTreeMap::new();
    let mut vertex_rims: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
    for (edge_id, kind) in &kinds {
        let edge = body
            .edge(*edge_id)
            .ok_or_else(|| refuse(Some(*edge_id), "the edge is missing"))?;
        let mut note = |v: VertexId| {
            let entry = vertex_faces.entry(v).or_default();
            match kind {
                Use::Boundary(f, _) => {
                    entry.insert(*f);
                }
                Use::Inner([a, b]) => {
                    entry.insert(a.0);
                    entry.insert(b.0);
                }
            }
        };
        note(edge.start);
        note(edge.end);
        if matches!(kind, Use::Boundary(..)) {
            for v in [edge.start, edge.end] {
                let list = vertex_rims.entry(v).or_default();
                if !list.contains(edge_id) {
                    list.push(*edge_id);
                }
            }
        }
    }
    let mut points: BTreeMap<(VertexId, usize), Vec3> = BTreeMap::new();
    for (vertex, around) in &vertex_faces {
        let v = body
            .vertex(*vertex)
            .ok_or_else(|| refuse(None, "a vertex of the faces is missing"))?;
        let at_rims = vertex_rims.get(vertex).cloned().unwrap_or_default();
        if at_rims.len() > 2 {
            return Err(refuse(
                at_rims.first().copied(),
                format!(
                    "the boundary of the faces passes through the corner at {} more than once: plate the faces on each side apart",
                    at(v.point)
                ),
            ));
        }
        for level in 0..2 {
            let mut surfaces: Vec<&Surface> = Vec::new();
            for f in around {
                if let Some(skin) = layered.get(*f).and_then(|l| l.skins.get(level)) {
                    surfaces.push(skin);
                }
            }
            let mut starts: Vec<Vec3> = Vec::new();
            for edge in &at_rims {
                if let Some((rim, _)) = rims.get(edge) {
                    surfaces.push(rim);
                }
                let (Some(t), Some(e)) = (traced.get(edge), body.edge(*edge)) else {
                    continue;
                };
                let row = t
                    .fitted
                    .get(level)
                    .ok_or_else(|| refuse(Some(*edge), "no row"))?;
                if e.start == *vertex {
                    starts.push(row.eval(t.own.0));
                }
                if e.end == *vertex {
                    starts.push(row.eval(t.own.1));
                }
            }
            let start = if starts.is_empty() {
                let f = around
                    .first()
                    .and_then(|f| layered.get(*f))
                    .ok_or_else(|| refuse(None, "a vertex without a face"))?;
                let (foot, normal) = f
                    .foot(v.point)
                    .ok_or_else(|| refuse(None, format!("no normal at {}", at(v.point))))?;
                foot + normal * f.levels.get(level).copied().unwrap_or(0.0)
            } else {
                starts.iter().fold(Vec3::ZERO, |s, p| s + *p) * (1.0 / starts.len() as f64)
            };
            // Each fitted skin is within the share of the tolerance of its
            // exact surface, so two that are tangent there are within
            // twice that of each other: a corner on both is held to that.
            let point = meet(&surfaces, start, 2.0 * SHARE * tol).ok_or_else(|| {
                refuse(
                    at_rims.first().copied(),
                    format!(
                        "the {} skin and the rims at the corner {} do not meet in one point",
                        if level == 0 { "back" } else { "front" },
                        at(v.point)
                    ),
                )
            })?;
            points.insert((*vertex, level), point);
        }
    }
    // The body.
    let mut out = Body::new(BodyKind::Solid);
    let mut vertex_ids: BTreeMap<(VertexId, usize), VertexId> = BTreeMap::new();
    for (key, p) in &points {
        vertex_ids.insert(*key, out.add_vertex(*p));
    }
    let vid = |v: VertexId, level: usize| -> Result<VertexId, KernelError> {
        vertex_ids
            .get(&(v, level))
            .copied()
            .ok_or_else(|| refuse(None, "a vertex of the plate is missing"))
    };
    // Skin edges, per edge of the faces and level.
    let mut skin_edges: BTreeMap<(EdgeId, usize), EdgeId> = BTreeMap::new();
    for (edge_id, kind) in &kinds {
        let edge = body
            .edge(*edge_id)
            .ok_or_else(|| refuse(Some(*edge_id), "the edge is missing"))?;
        let closed = edge.start == edge.end;
        let rows: [(Curve3, Vec<Vec3>); 2] = match kind {
            Use::Boundary(..) => {
                let t = traced
                    .get(edge_id)
                    .ok_or_else(|| refuse(Some(*edge_id), "an edge was not traced"))?;
                let [lo, hi] = &t.fitted;
                let [rlo, rhi] = &t.rows;
                [
                    (simplest(rlo, lo, tol), rlo.clone()),
                    (simplest(rhi, hi, tol), rhi.clone()),
                ]
            }
            Use::Inner([a, b]) => {
                let (fa, fb) = (
                    layered
                        .get(a.0)
                        .ok_or_else(|| refuse(None, "face missing"))?,
                    layered
                        .get(b.0)
                        .ok_or_else(|| refuse(None, "face missing"))?,
                );
                inner_rows(*edge_id, edge, fa, fb, levels, tol)?
            }
        };
        for (level, (curve, _)) in rows.into_iter().enumerate() {
            let (start, end) = (vid(edge.start, level)?, vid(edge.end, level)?);
            let (Some(ps), Some(pe)) = (out.vertex(start), out.vertex(end)) else {
                return Err(refuse(Some(*edge_id), "a vertex of the plate is missing"));
            };
            let range = range_between(&curve, ps.point, pe.point, closed, tol).ok_or_else(|| {
                refuse(
                    Some(*edge_id),
                    format!(
                        "the {} edge along the edge from {} does not reach the corners of the plate: the corner lies further past the edge's end than its tangent planes put it, by more than the margin the edge is traced beyond",
                        if level == 0 { "back" } else { "front" },
                        at(edge.curve.eval(edge.range.0))
                    ),
                )
            })?;
            skin_edges.insert((*edge_id, level), out.add_edge(curve, range, start, end));
        }
    }
    // Edges across the plate at the corners of its boundary: from the
    // front vertex to the back one, where the two rims meet.
    let mut across: BTreeMap<VertexId, EdgeId> = BTreeMap::new();
    for (vertex, at_rims) in &vertex_rims {
        let (front, back) = (vid(*vertex, 1)?, vid(*vertex, 0)?);
        let (Some(pf), Some(pb)) = (out.vertex(front), out.vertex(back)) else {
            return Err(refuse(None, "a vertex of the plate is missing"));
        };
        let (pf, pb) = (pf.point, pb.point);
        let mut surfaces: Vec<&Surface> = Vec::new();
        for edge in at_rims {
            if let Some((rim, _)) = rims.get(edge) {
                surfaces.push(rim);
            }
        }
        // Where the rims say nothing more (one rim, or two that continue
        // each other) the plane square to the chord keeps the point on the
        // rim nearest the chord.
        let first = at_rims
            .first()
            .and_then(|e| boundaries.iter().find(|b| b.edge_id == *e))
            .ok_or_else(|| refuse(None, "a corner without a rim"))?;
        let chord = pb - pf;
        let mut at_param = |s: f64| -> Result<Vec<Vec3>, KernelError> {
            if s <= 0.0 {
                return Ok(vec![pf]);
            }
            if s >= 1.0 {
                return Ok(vec![pb]);
            }
            let start = pf + chord * s;
            let plane = Frame::from_z(start, chord)
                .map(|f| Surface::Plane(Plane::new(f)))
                .ok_or_else(|| refuse(None, "the plate is thinner than a point at a corner"))?;
            let mut list = surfaces.clone();
            list.push(&plane);
            meet(&list, start, SHARE * tol)
                .map(|p| vec![p])
                .ok_or_else(|| {
                    refuse(
                        first.edge_id.into(),
                        format!(
                            "the rims at the corner {} do not meet across the plate",
                            at(start)
                        ),
                    )
                })
        };
        let params: Vec<f64> = (0..=4).map(|k| f64::from(k) / 4.0).collect();
        let (params, rows) = trace(
            params,
            1,
            Some(first.edge_id),
            "edge across a corner",
            &mut at_param,
            tol,
        )?;
        let row = rows
            .first()
            .ok_or_else(|| refuse(None, "no points across"))?;
        let spline = fit(&params, row)?;
        let curve = simplest(row, &spline, tol);
        let range = range_between(&curve, pf, pb, false, tol).ok_or_else(|| {
            refuse(
                Some(first.edge_id),
                "the edge across a corner does not run from front to back",
            )
        })?;
        across.insert(*vertex, out.add_edge(curve, range, front, back));
    }
    // Faces.
    let mut made: Vec<Made> = Vec::new();
    let mut faces_out: Vec<FaceId> = Vec::new();
    for (id, l) in chosen.iter().zip(&layered) {
        for level in [1usize, 0] {
            let mut loops = Vec::with_capacity(l.face.loops.len());
            for loop_ in l.face.loops.iter().filter_map(|x| body.loop_(*x)) {
                let mut coedges: Vec<Coedge> = Vec::with_capacity(loop_.coedges.len());
                for c in &loop_.coedges {
                    let edge = skin_edges
                        .get(&(c.edge, level))
                        .copied()
                        .ok_or_else(|| refuse(Some(c.edge), "a skin edge is missing"))?;
                    coedges.push(Coedge {
                        edge,
                        reversed: if level == 1 { c.reversed } else { !c.reversed },
                    });
                }
                if level == 0 {
                    coedges.reverse();
                }
                loops.push(out.add_loop(coedges));
            }
            let skin = l
                .skins
                .get(level)
                .cloned()
                .ok_or_else(|| refuse(None, "a skin is missing"))?;
            let reversed = if level == 1 {
                l.face.reversed
            } else {
                !l.face.reversed
            };
            held_by(&out, &loops, l, level, *id, tolerance)?;
            faces_out.push(out.add_face(skin, reversed, loops));
            made.push(if level == 1 {
                Made::Front(*id)
            } else {
                Made::Back(*id)
            });
        }
    }
    for boundary in &boundaries {
        let e = boundary.edge;
        let forward = boundary.along > 0.0;
        let (v_start, v_end) = if forward {
            (e.start, e.end)
        } else {
            (e.end, e.start)
        };
        let get = |key: (EdgeId, usize)| {
            skin_edges
                .get(&key)
                .copied()
                .ok_or_else(|| refuse(Some(boundary.edge_id), "a skin edge is missing"))
        };
        let (front, back) = (get((boundary.edge_id, 1))?, get((boundary.edge_id, 0))?);
        let down = |v: VertexId| {
            across
                .get(&v)
                .copied()
                .ok_or_else(|| refuse(Some(boundary.edge_id), "an edge across a corner is missing"))
        };
        let coedges = vec![
            Coedge {
                edge: front,
                reversed: forward,
            },
            Coedge {
                edge: down(v_start)?,
                reversed: false,
            },
            Coedge {
                edge: back,
                reversed: !forward,
            },
            Coedge {
                edge: down(v_end)?,
                reversed: true,
            },
        ];
        let loop_ = out.add_loop(coedges);
        let (surface, reversed) = rims
            .get(&boundary.edge_id)
            .cloned()
            .ok_or_else(|| refuse(Some(boundary.edge_id), "a rim is missing"))?;
        faces_out.push(out.add_face(surface, reversed, vec![loop_]));
        made.push(Made::Rim(boundary.edge_id));
    }
    out.add_shell(faces_out);
    let defects = check(&out, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    Ok(Plate {
        body: out,
        faces: made,
    })
}

/// Samples along each edge of a skin checked to lie where the skin is
/// held to the tolerance ([`held_by`]), its ends included.
const HELD_SAMPLES: usize = 16;

/// That every vertex and every sample of the edges of the skin at
/// `level` of `face` (its loops `loops` in `out`) lies within the region
/// its offset was held to the tolerance in ([`Layered::new`],
/// `craft3d_geom::offset::Held`): the held region is chosen with a
/// margin over the furthest corner, and this makes it a guarantee. Each
/// point is put on the face's surface continued (the skin keeps its
/// parameters) and measured from its domain as the fit was. A skin that
/// is the face's surface itself, or an analytic offset, is exact
/// everywhere and not checked.
fn held_by(
    out: &Body,
    loops: &[craft3d_topo::LoopId],
    face: &Layered<'_>,
    level: usize,
    id: FaceId,
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    let at_level = face.levels.get(level).copied().unwrap_or(0.0);
    if at_level == 0.0 || !face.held.is_finite() {
        return Ok(());
    }
    let Some((nurbs, _)) =
        crate::offset_face::reached_each(&face.face.surface, [0.0; 4], tolerance)
    else {
        // An analytic surface: its offsets are exact.
        return Ok(());
    };
    let held = craft3d_geom::offset::Held {
        around: nurbs.domain(),
        reach: face.held,
    };
    for lp in loops {
        let Some(l) = out.loop_(*lp) else { continue };
        for c in &l.coedges {
            let Some(edge) = out.edge(c.edge) else {
                continue;
            };
            let (t0, t1) = edge.range;
            for k in 0..=HELD_SAMPLES {
                let t = t0 + (t1 - t0) * k as f64 / HELD_SAMPLES as f64;
                let p = edge.curve.eval(t);
                let (u, v) = face.reference.project(p).map_err(|_| {
                    refuse(
                        None,
                        format!(
                            "a point of the skin of face {} at {} cannot be put on its surface",
                            id.0,
                            at(p)
                        ),
                    )
                })?;
                let length = held.length(&nurbs, u, v);
                if !(length <= held.reach) {
                    let which = if k == 0 || k == HELD_SAMPLES {
                        "the plate's corner"
                    } else {
                        "a point of the plate's edge"
                    };
                    return Err(refuse(
                        None,
                        format!(
                            "{which} at {} lies {} along the surface past the border of face {}, further than its skin is held to the tolerance ({}): the joints reach further past the face than the plate allows for",
                            at(p),
                            short(length),
                            id.0,
                            short(held.reach)
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The faces must hang together across the edges they share.
fn connected(layered: &[Layered<'_>], kinds: &BTreeMap<EdgeId, Use>) -> Result<(), KernelError> {
    let mut group: Vec<usize> = (0..layered.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        for _ in 0..group.len() {
            let Some(&up) = group.get(i) else { break };
            if up == i {
                break;
            }
            i = up;
        }
        i
    }
    for kind in kinds.values() {
        if let Use::Inner([a, b]) = kind {
            let (ra, rb) = (root(&mut group, a.0), root(&mut group, b.0));
            if let Some(slot) = group.get_mut(ra.max(rb)) {
                *slot = ra.min(rb);
            }
        }
    }
    let first = root(&mut group, 0);
    if (0..layered.len()).any(|i| root(&mut group, i) != first) {
        return Err(refuse(
            None,
            "the faces do not hang together across shared edges: plate each piece apart",
        ));
    }
    Ok(())
}

/// Points an edge between two faces is looked at for a crease before the
/// plate is traced.
const CREASE_SAMPLES: usize = 8;

/// The foot of `p` on face `a` and its normal there; an error when faces
/// `a` and `b` are not tangent at `p` within what the levels allow (their
/// offsets would part by more than a quarter of the tolerance).
fn creased(
    edge_id: EdgeId,
    p: Vec3,
    a: &Layered<'_>,
    b: &Layered<'_>,
    levels: (f64, f64),
    tol: f64,
) -> Result<(Vec3, Vec3), KernelError> {
    let largest = levels.0.abs().max(levels.1.abs());
    let (foot, na) = a
        .foot(p)
        .ok_or_else(|| refuse(Some(edge_id), format!("no normal at {}", at(p))))?;
    let (_, nb) = b
        .foot(p)
        .ok_or_else(|| refuse(Some(edge_id), format!("no normal at {}", at(p))))?;
    // The rule of the face operations for faces that are tangent only so
    // far: their offsets may part by at most the share of the tolerance a
    // turned normal may move an offset by.
    let apart = na.distance(nb);
    if !(apart * largest <= TURN_SHARE * tol) {
        return Err(refuse(
            Some(edge_id),
            format!(
                "the faces of the plate meet at a crease at {} ({} degrees): {TURN_TOO_FAR}; plate them apart and join them",
                at(p),
                short((2.0 * (0.5 * apart).clamp(0.0, 1.0).asin()).to_degrees())
            ),
        ));
    }
    Ok((foot, na))
}

/// The two skin edges along an edge between two faces of the plate: the
/// edge offset along their (shared) normal to each level. Refused where
/// the faces are not tangent within what the levels allow.
fn inner_rows(
    edge_id: EdgeId,
    edge: &Edge,
    a: &Layered<'_>,
    b: &Layered<'_>,
    levels: (f64, f64),
    tol: f64,
) -> Result<[(Curve3, Vec<Vec3>); 2], KernelError> {
    let mut at_param = |t: f64| -> Result<Vec<Vec3>, KernelError> {
        let (foot, na) = creased(edge_id, edge.curve.eval(t), a, b, levels, tol)?;
        Ok(vec![foot + na * levels.0, foot + na * levels.1])
    };
    let (from, to) = edge.range;
    let params = steps(from, to, turned_steps(&edge.curve, from, to, FIRST_STEPS));
    let (params, rows) = trace(
        params,
        2,
        Some(edge_id),
        "edge between two faces",
        &mut at_param,
        tol,
    )?;
    let mut out: Vec<(Curve3, Vec<Vec3>)> = Vec::with_capacity(2);
    for row in rows {
        let spline = fit(&params, &row)?;
        out.push((simplest(&row, &spline, tol), row));
    }
    let [lo, hi]: [(Curve3, Vec<Vec3>); 2] = out
        .try_into()
        .map_err(|_| refuse(Some(edge_id), "two rows expected"))?;
    Ok([lo, hi])
}

/// The most a traced curve's tangent turns over one first step (radians):
/// a long arc starts with as many steps as its turn needs, so that the
/// spline through the first points is near the curve everywhere and the
/// middles added where it is not stay few. (Sixteen steps over a 270° arc
/// left the spline so far off that refining in places never settled.)
const STEP_TURN: f64 = 0.1;

/// Samples of the tangent for [`turned_steps`].
const TURN_SAMPLES: usize = 64;

/// How many first steps a curve needs over `a..b`: at least `least`, and
/// enough that each turns the tangent by at most [`STEP_TURN`] (the turn
/// summed over [`TURN_SAMPLES`] samples), at most an eighth of
/// [`MAX_STATIONS`].
fn turned_steps(curve: &Curve3, a: f64, b: f64, least: usize) -> usize {
    let mut turn = 0.0;
    let mut last: Option<Vec3> = None;
    for k in 0..=TURN_SAMPLES {
        let t = a + (b - a) * (k as f64 / TURN_SAMPLES as f64);
        let Some(d) = curve.derivatives(t).1.normalized() else {
            continue;
        };
        if let Some(prev) = last {
            turn += prev.dot(d).clamp(-1.0, 1.0).acos();
        }
        last = Some(d);
    }
    let needed = (turn / STEP_TURN).ceil();
    let needed = if needed.is_finite() && needed > 0.0 {
        needed.min((MAX_STATIONS / 8) as f64) as usize
    } else {
        0
    };
    needed.max(least)
}

fn steps(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..=n)
        .map(|k| a + (b - a) * (k as f64 / n as f64))
        .collect()
}

/// A spline edge's curve continued over `reach` as the function its end
/// spans are, re-expressed so that it is evaluated inside its own domain
/// (`NurbsCurve::extended`); `None` for the other kinds of curve (each
/// continued exactly by its own closed form) and where the domain already
/// holds the reach. Evaluating a spline past its domain directly scatters
/// its points when an end span is short against the reach: the sea
/// kayak's sheer plate (2026-10-28) traced its end edge, whose first span
/// is a small part of it, a whole edge length past it, where the edge's
/// points scattered by 1e-5 mm along it and the rim's stations with them,
/// and no refinement fitted a spline through them within a quarter of
/// the tolerance (1e-6 mm).
fn continued_past(curve: &Curve3, reach: (f64, f64)) -> Option<Curve3> {
    let Curve3::Nurbs(spline) = curve else {
        return None;
    };
    let (t0, t1) = spline.domain();
    if !(reach.0 < t0 || reach.1 > t1) {
        return None;
    }
    spline.extended(reach).ok().map(Curve3::Nurbs)
}

/// The curve to evaluate an edge at `t` on: its own inside its domain
/// (every point there as it always was), `longer` past it.
fn curve_at<'c>(own: &'c Curve3, longer: Option<&'c Curve3>, t: f64) -> &'c Curve3 {
    match (own, longer) {
        (Curve3::Nurbs(spline), Some(longer)) => {
            let (t0, t1) = spline.domain();
            if t < t0 || t > t1 { longer } else { own }
        }
        _ => own,
    }
}

/// The corners of a boundary edge traced over its range and its reach,
/// and its rim with whether the rim's outward normal is against its
/// surface's.
fn trace_boundary(
    boundary: &Boundary<'_, '_>,
    face: &Layered<'_>,
    (reach_before, reach_after): (f64, f64),
    tolerance: &Tolerance,
) -> Result<(Traced, (Surface, bool)), KernelError> {
    let tol = tolerance.linear;
    let edge = boundary.edge;
    let edge_id = boundary.edge_id;
    let (a, b) = edge.range;
    let closed = edge.start == edge.end;
    // The reach past each end, in the edge's parameter.
    let speed = |t: f64| edge.curve.derivatives(t).1.length();
    let past = |t: f64, length: f64| {
        let s = speed(t);
        if s > 0.0 && s.is_finite() {
            length / s
        } else {
            0.0
        }
    };
    let (before, after) = if closed {
        (0.0, 0.0)
    } else {
        (past(a, reach_before), past(b, reach_after))
    };
    // Past its ends the edge's curve goes on as itself, evaluated where
    // that is well conditioned.
    let longer = continued_past(&edge.curve, (a - before, b + after));
    let past_curve = longer.as_ref().unwrap_or(&edge.curve);
    let curve = |t: f64| curve_at(&edge.curve, longer.as_ref(), t);
    let mut params = Vec::new();
    if before > 0.0 {
        params.extend(steps(
            a - before,
            a,
            turned_steps(past_curve, a - before, a, REACH_STEPS),
        ));
        params.pop();
    }
    params.extend(steps(a, b, turned_steps(&edge.curve, a, b, FIRST_STEPS)));
    if after > 0.0 {
        params.extend(
            steps(
                b,
                b + after,
                turned_steps(past_curve, b, b + after, REACH_STEPS),
            )
            .into_iter()
            .skip(1),
        );
    }
    // Which skins face the neighbour's material, from the middle.
    let middle = station(edge, 0.5 * (a + b), boundary.along, face)
        .ok_or_else(|| refuse(Some(edge_id), "the face has no normal along the edge"))?;
    let sides = match &boundary.near {
        None => None,
        Some(near) => Some(sides(edge_id, near, &middle, tolerance)?),
    };
    let levels = face.levels;
    let mut at_param = |t: f64| -> Result<Vec<Vec3>, KernelError> {
        let inside = t >= a - 1e-12 * (b - a).abs() && t <= b + 1e-12 * (b - a).abs();
        let st = station_on(curve(t), t, boundary.along, face).ok_or_else(|| {
            refuse(
                Some(edge_id),
                format!("the face has no normal at {}", at(curve(t).eval(t))),
            )
        })?;
        let [lo, hi] = corners(boundary, face, &st, sides, levels, inside, tolerance)?;
        // A corner past the edge's end that lands on the border of a
        // skin's parameters ran off its continuation: refused, never
        // taken.
        for (level, p) in [(0usize, lo), (1, hi)] {
            if !inside
                && face
                    .skins
                    .get(level)
                    .is_some_and(|skin| at_border(skin, p, tol))
            {
                return Err(refuse(Some(edge_id), PAST_REACH));
            }
        }
        Ok(vec![lo, hi])
    };
    let (params, rows) = trace(params, 2, Some(edge_id), "rim", &mut at_param, tol)?;
    let fitted: Vec<NurbsCurve> = rows
        .iter()
        .map(|row| fit(&params, row))
        .collect::<Result<_, _>>()?;
    let [lo_fit, hi_fit]: [NurbsCurve; 2] = fitted
        .try_into()
        .map_err(|_| refuse(Some(edge_id), "two rows expected"))?;
    let [lo_row, hi_row]: [Vec<Vec3>; 2] = rows
        .try_into()
        .map_err(|_| refuse(Some(edge_id), "two rows expected"))?;
    let surface = match (&boundary.joint, &boundary.near, sides) {
        (Joint::Over(_), Some(near), Some(s)) => near
            .layered
            .skins
            .get(s.near_away)
            .cloned()
            .ok_or_else(|| refuse(Some(edge_id), "a skin is missing"))?,
        (Joint::Under(_), Some(near), Some(s)) => near
            .layered
            .skins
            .get(s.near_toward)
            .cloned()
            .ok_or_else(|| refuse(Some(edge_id), "a skin is missing"))?,
        (Joint::Onto(_), _, _) => boundary
            .end_surface()
            .cloned()
            .ok_or_else(|| refuse(Some(edge_id), "an onto end lost its surface"))?,
        _ => ruled_rim([&hi_row, &lo_row], [&hi_fit, &lo_fit], tol)?,
    };
    // Outward: away from the material, against the inward direction.
    let mid_rim = (middle.point + middle.normal * levels[0])
        .lerp(middle.point + middle.normal * levels[1], 0.5);
    let probe = {
        let k = hi_row.len() / 2;
        match (hi_row.get(k), lo_row.get(k)) {
            (Some(h), Some(l)) => h.lerp(*l, 0.5),
            _ => mid_rim,
        }
    };
    let normal = surface_normal(&surface, probe)
        .ok_or_else(|| refuse(Some(edge_id), "the rim has no normal"))?;
    let reversed = normal.dot(-middle.inward) < 0.0;
    Ok((
        Traced {
            rows: [lo_row, hi_row],
            fitted: [lo_fit, hi_fit],
            own: (a, b),
        },
        (surface, reversed),
    ))
}

/// The corners of a rim at one station by the tangent planes alone: the
/// line of a square or bevelled end, and the meeting of the skins' tangent
/// planes in the station's plane for a joint (the start [`corners`] takes
/// for its Newton steps).
fn flat_corners(
    boundary: &Boundary<'_, '_>,
    st: &Station,
    sides: Option<Sides>,
    levels: [f64; 2],
    angular: f64,
) -> Option<[Vec3; 2]> {
    let level = |k: usize| levels.get(k).copied().unwrap_or(0.0);
    match (&boundary.joint, &boundary.near, sides) {
        (Joint::Square, _, _) => Some([
            st.point + st.normal * level(0),
            st.point + st.normal * level(1),
        ]),
        (Joint::Bevel(angle), _, _) => {
            let (s, c) = angle.sin_cos();
            let d = st.normal * c - st.inward * s;
            Some([st.point + d * (level(0) / c), st.point + d * (level(1) / c)])
        }
        (Joint::Onto(_), _, _) => {
            let (q, m) = foot_on(boundary.end_surface()?, st.point)?;
            Some([
                onto_start(st, q, m, level(0), angular)?,
                onto_start(st, q, m, level(1), angular)?,
            ])
        }
        (joint, Some(near), Some(s)) => {
            let (n_near, _, _, _) = near_frame(near, st.point)?;
            let seen = st.inward.dot(n_near);
            if !(seen.abs() > 0.0) {
                return None;
            }
            let near_level = |k: usize| near.layered.levels.get(k).copied().unwrap_or(0.0);
            let cross = |own: usize, other: usize| {
                let (a, c) = (level(own), near_level(other));
                let x = (c - a * st.normal.dot(n_near)) / seen;
                st.point + st.inward * x + st.normal * a
            };
            let (toward, away) = match joint {
                Joint::Mitre(_) => (
                    cross(s.own_toward, s.near_toward),
                    cross(s.own_away, s.near_away),
                ),
                Joint::Touch(_) => {
                    let toward = cross(s.own_toward, s.near_toward);
                    let away = toward + st.normal * (level(s.own_away) - level(s.own_toward));
                    (toward, away)
                }
                Joint::Over(_) => (
                    cross(s.own_toward, s.near_away),
                    cross(s.own_away, s.near_away),
                ),
                Joint::Under(_) => (
                    cross(s.own_toward, s.near_toward),
                    cross(s.own_away, s.near_toward),
                ),
                Joint::Square | Joint::Bevel(_) | Joint::Onto(_) => return None,
            };
            Some(if s.own_toward == 1 {
                [away, toward]
            } else {
                [toward, away]
            })
        }
        _ => None,
    }
}

/// How far an edge spans: the diagonal of the box of points along it. A
/// surface an end lies on is continued by this and the reach, so that the
/// whole end lies on it however small the face it was given as.
fn span(edge: &Edge) -> f64 {
    let (a, z) = edge.range;
    let points = (0..=16).map(|k| edge.curve.eval(a + (z - a) * f64::from(k) / 16.0));
    craft3d_math::Bbox3::from_points(points).diagonal()
}

/// The nearest point of `surface` to `p` and its normal there.
fn foot_on(surface: &Surface, p: Vec3) -> Option<(Vec3, Vec3)> {
    let (u, v) = surface.project(p).ok()?;
    let normal = surface.normal(u, v)?;
    let foot = surface.eval(u, v);
    (foot.is_finite() && normal.is_finite()).then_some((foot, normal))
}

/// Where the skin at `height` meets the tangent plane `(q, m)` of the
/// surface an edge ends on, in the plane square to the edge, by the skin's
/// tangent plane: `C + height N + w b` with `w` from the plane. `None` when
/// the surface holds the plate's inward direction (it runs along the
/// plate and cuts no end).
fn onto_start(st: &Station, q: Vec3, m: Vec3, height: f64, angular: f64) -> Option<Vec3> {
    let across = st.inward.dot(m);
    if !(across.abs() > angular.sin()) {
        return None;
    }
    let base = st.point + st.normal * height;
    let w = (q - base).dot(m) / across;
    let p = base + st.inward * w;
    p.is_finite().then_some(p)
}

/// An end of a boundary edge: its vertex, the station there, the rim's
/// corners there by the tangent planes, and the way out past the end.
type End = (VertexId, Station, [Vec3; 2], Vec3);

/// The point where three planes (a point and a normal each) meet; `None`
/// when two of them are parallel within the angular tolerance's sine.
fn three_planes(planes: [(Vec3, Vec3); 3], angular: f64) -> Option<Vec3> {
    let [(p1, n1), (p2, n2), (p3, n3)] = planes;
    let det = n1.dot(n2.cross(n3));
    if !(det.abs() > angular.sin().max(1e-12)) {
        return None;
    }
    let point = (n2.cross(n3) * n1.dot(p1) + n3.cross(n1) * n2.dot(p2) + n1.cross(n2) * n3.dot(p3))
        * (1.0 / det);
    point.is_finite().then_some(point)
}

/// How far each boundary edge is traced past its start and its end, as
/// lengths: per corner of the faces where two rims meet, how far past the
/// edge's end the plate's corner lies by the tangent planes (each skin's,
/// and each rim's through its two corners at the end and the edge's
/// tangent), plus a margin of twice the thicknesses so the curves run past
/// it. Where the rims continue each other (their planes do not meet in a
/// point) the margin alone.
fn reaches(
    boundaries: &[Boundary<'_, '_>],
    layered: &[Layered<'_>],
    thickness: f64,
    tolerance: &Tolerance,
) -> Result<BTreeMap<EdgeId, (f64, f64)>, KernelError> {
    let margin = 2.0
        * (thickness
            + boundaries
                .iter()
                .filter_map(|b| b.joint.neighbour().map(|n| n.thickness))
                .fold(0.0_f64, f64::max));
    // An end on a surface starts on it: the edge must lie within the
    // margin of it (a wrong datum is said, not traced into).
    for b in boundaries {
        let Some(end) = b.end_surface() else { continue };
        let (a, z) = b.edge.range;
        for t in [a, 0.5 * (a + z), z] {
            let p = b.edge.curve.eval(t);
            let away = foot_on(end, p).map_or(f64::INFINITY, |(q, _)| q.distance(p));
            if !(away <= margin) {
                return Err(refuse(
                    Some(b.edge_id),
                    format!(
                        "the surface this edge ends on is {} away from the edge at {}: an onto end lies on a surface through (or within {} of) the edge",
                        short(away),
                        at(p),
                        short(margin)
                    ),
                ));
            }
        }
    }
    // Per boundary edge and end (false: start, true: end): the station
    // there, its corners by the tangent planes and the way out past it.
    let mut ends: BTreeMap<(EdgeId, bool), End> = BTreeMap::new();
    for b in boundaries {
        let face = layered
            .get(b.face)
            .ok_or_else(|| refuse(None, "a face of the plate is missing"))?;
        if b.edge.start == b.edge.end {
            continue;
        }
        let (a, z) = b.edge.range;
        let middle = station(b.edge, 0.5 * (a + z), b.along, face)
            .ok_or_else(|| refuse(Some(b.edge_id), "the face has no normal along the edge"))?;
        let sides = match &b.near {
            None => None,
            Some(near) => Some(sides(b.edge_id, near, &middle, tolerance)?),
        };
        for (is_end, t, vertex) in [(false, a, b.edge.start), (true, z, b.edge.end)] {
            let st = station(b.edge, t, b.along, face).ok_or_else(|| {
                refuse(
                    Some(b.edge_id),
                    "the face has no normal at an end of the edge",
                )
            })?;
            let corners =
                flat_corners(b, &st, sides, face.levels, tolerance.angular).ok_or_else(|| {
                    refuse(
                        Some(b.edge_id),
                        if matches!(b.joint, Joint::Onto(_)) {
                            format!(
                                "the surface the edge ends on runs along the plate at {} (across the edge it holds the plate's own direction): it cuts no end",
                                at(st.point)
                            )
                        } else {
                            format!("the joint has no corner at {}", at(st.point))
                        },
                    )
                })?;
            let d = b.edge.curve.derivatives(t).1;
            let out = (if is_end { d } else { -d })
                .normalized()
                .unwrap_or(Vec3::ZERO);
            ends.insert((b.edge_id, is_end), (vertex, st, corners, out));
        }
    }
    let mut out: BTreeMap<EdgeId, (f64, f64)> = boundaries
        .iter()
        .map(|b| {
            let closed = b.edge.start == b.edge.end;
            (
                b.edge_id,
                if closed { (0.0, 0.0) } else { (margin, margin) },
            )
        })
        .collect();
    let keys: Vec<(EdgeId, bool)> = ends.keys().copied().collect();
    for (i, first) in keys.iter().enumerate() {
        for second in keys.iter().skip(i + 1) {
            let (Some(e1), Some(e2)) = (ends.get(first), ends.get(second)) else {
                continue;
            };
            if e1.0 != e2.0 || first.0 == second.0 {
                continue;
            }
            let rim_plane = |e: &End| {
                let [lo, hi] = e.2;
                (hi - lo).cross(e.1.tangent).normalized().map(|n| (lo, n))
            };
            let (Some(r1), Some(r2)) = (rim_plane(e1), rim_plane(e2)) else {
                continue;
            };
            let mut past = [0.0_f64; 2];
            for level in 0..2 {
                let at_level = e1.2.get(level).copied().unwrap_or(e1.1.point);
                let Some(corner) =
                    three_planes([(at_level, e1.1.normal), r1, r2], tolerance.angular)
                else {
                    continue;
                };
                for (k, e) in [e1, e2].into_iter().enumerate() {
                    if let Some(slot) = past.get_mut(k) {
                        *slot = slot.max((corner - e.1.point).dot(e.3));
                    }
                }
            }
            for (key, extra) in [(first, past[0]), (second, past[1])] {
                if let Some((before, after)) = out.get_mut(&key.0) {
                    let slot = if key.1 { after } else { before };
                    *slot = slot.max(extra.max(0.0) + margin);
                }
            }
        }
    }
    Ok(out)
}

/// Which skin of each plate faces the other's material: indices into the
/// levels (0 back, 1 front).
#[derive(Clone, Copy, Debug)]
struct Sides {
    own_toward: usize,
    own_away: usize,
    near_toward: usize,
    near_away: usize,
}

fn sides(
    edge_id: EdgeId,
    near: &Near<'_>,
    st: &Station,
    tolerance: &Tolerance,
) -> Result<Sides, KernelError> {
    let (n_near, in_near, off_surface, off_edge) = near_frame(near, st.point).ok_or_else(|| {
        refuse(
            Some(edge_id),
            format!("the neighbour's face has no normal near {}", at(st.point)),
        )
    })?;
    let tol = tolerance.linear;
    if !(off_surface <= tol && off_edge <= tol) {
        return Err(refuse(
            Some(edge_id),
            format!(
                "the edge does not run along the boundary of the neighbour's face: at {} it is {} away",
                at(st.point),
                short(off_surface.max(off_edge))
            ),
        ));
    }
    let toward = in_near.dot(st.normal);
    let seen = st.inward.dot(n_near);
    if !(toward.abs() > tolerance.angular.sin() && seen.abs() > tolerance.angular.sin()) {
        return Err(refuse(
            Some(edge_id),
            "the plates continue each other in line along the edge: a joint between them has no corner; give the edge a square or bevelled end",
        ));
    }
    let own_toward = usize::from(toward > 0.0);
    let near_toward = usize::from(seen > 0.0);
    Ok(Sides {
        own_toward,
        own_away: 1 - own_toward,
        near_toward,
        near_away: 1 - near_toward,
    })
}

/// The points of the rim on the back and the front skin at one station.
fn corners(
    boundary: &Boundary<'_, '_>,
    face: &Layered<'_>,
    st: &Station,
    sides_mid: Option<Sides>,
    levels: [f64; 2],
    inside: bool,
    tolerance: &Tolerance,
) -> Result<[Vec3; 2], KernelError> {
    let tol = tolerance.linear;
    let edge_id = boundary.edge_id;
    let lost = |p: Vec3| {
        refuse(
            Some(edge_id),
            format!("the rim could not be put on the skins near {}", at(p)),
        )
    };
    let skin = |level: usize| {
        face.skins
            .get(level)
            .ok_or_else(|| refuse(Some(edge_id), "no skin"))
    };
    let snap = |level: usize, p: Vec3| -> Result<Vec3, KernelError> {
        meet(&[skin(level)?], p, SHARE * tol).ok_or_else(|| lost(p))
    };
    let plane = st.plane().ok_or_else(|| lost(st.point))?;
    match (&boundary.joint, &boundary.near, sides_mid) {
        (Joint::Square, _, _) => Ok([
            snap(0, st.point + st.normal * levels[0])?,
            snap(1, st.point + st.normal * levels[1])?,
        ]),
        (Joint::Bevel(angle), _, _) => {
            let (s, c) = angle.sin_cos();
            let d = st.normal * c - st.inward * s;
            let side = Frame::from_z(st.point, d.cross(st.tangent))
                .map(|f| Surface::Plane(Plane::new(f)))
                .ok_or_else(|| lost(st.point))?;
            let mut out = [Vec3::ZERO; 2];
            for (level, slot) in out.iter_mut().enumerate() {
                let height = levels.get(level).copied().unwrap_or(0.0);
                let start = st.point + d * (height / c);
                *slot = meet(&[skin(level)?, &plane, &side], start, SHARE * tol)
                    .ok_or_else(|| lost(start))?;
            }
            Ok(out)
        }
        (Joint::Onto(_), _, _) => {
            let end = boundary.end_surface().ok_or_else(|| lost(st.point))?;
            let (q, m) = foot_on(end, st.point).ok_or_else(|| lost(st.point))?;
            let mut out = [Vec3::ZERO; 2];
            for (level, slot) in out.iter_mut().enumerate() {
                let height = levels.get(level).copied().unwrap_or(0.0);
                let start = onto_start(st, q, m, height, tolerance.angular).ok_or_else(|| {
                    refuse(
                        Some(edge_id),
                        format!(
                            "the surface the edge ends on runs along the plate at {} (across the edge it holds the plate's own direction): it cuts no end",
                            at(st.point)
                        ),
                    )
                })?;
                *slot = meet(&[skin(level)?, end, &plane], start, SHARE * tol)
                    .ok_or_else(|| lost(start))?;
            }
            Ok(out)
        }
        (joint, Some(near), Some(s)) => {
            let (n_near, _, off_surface, off_edge) =
                near_frame(near, st.point).ok_or_else(|| lost(st.point))?;
            if inside && !(off_surface <= tol && off_edge <= tol) {
                return Err(refuse(
                    Some(edge_id),
                    format!(
                        "the edge leaves the boundary of the neighbour's face at {} ({} away)",
                        at(st.point),
                        short(off_surface.max(off_edge))
                    ),
                ));
            }
            let seen = st.inward.dot(n_near);
            if !(seen.abs() > tolerance.angular.sin()) || (seen > 0.0) != (s.near_toward == 1) {
                return Err(refuse(
                    Some(edge_id),
                    format!(
                        "the plates turn through being in line with each other along the edge, at {}: split the edge there",
                        at(st.point)
                    ),
                ));
            }
            let near_level = |k: usize| near.layered.levels.get(k).copied().unwrap_or(0.0);
            let near_skin = |k: usize| {
                near.layered
                    .skins
                    .get(k)
                    .ok_or_else(|| refuse(Some(edge_id), "no skin"))
            };
            // Where this plate's skin at `own` meets the neighbour's at
            // `other`, in the plane square to the edge; from the meeting of
            // their tangent planes there.
            let cross = |own: usize, other: usize| -> Result<Vec3, KernelError> {
                let (a, c) = (levels.get(own).copied().unwrap_or(0.0), near_level(other));
                let x = (c - a * st.normal.dot(n_near)) / seen;
                let start = st.point + st.inward * x + st.normal * a;
                meet(&[skin(own)?, near_skin(other)?, &plane], start, SHARE * tol)
                    .ok_or_else(|| lost(start))
            };
            let (toward, away) = match joint {
                Joint::Mitre(_) => (
                    cross(s.own_toward, s.near_toward)?,
                    cross(s.own_away, s.near_away)?,
                ),
                Joint::Touch(_) => {
                    let toward = cross(s.own_toward, s.near_toward)?;
                    let (foot, normal) = face.foot(toward).ok_or_else(|| lost(toward))?;
                    let away = snap(
                        s.own_away,
                        foot + normal * levels.get(s.own_away).copied().unwrap_or(0.0),
                    )?;
                    (toward, away)
                }
                Joint::Over(_) => (
                    cross(s.own_toward, s.near_away)?,
                    cross(s.own_away, s.near_away)?,
                ),
                Joint::Under(_) => (
                    cross(s.own_toward, s.near_toward)?,
                    cross(s.own_away, s.near_toward)?,
                ),
                Joint::Square | Joint::Bevel(_) | Joint::Onto(_) => return Err(lost(st.point)),
            };
            Ok(if s.own_toward == 1 {
                [away, toward]
            } else {
                [toward, away]
            })
        }
        _ => Err(refuse(Some(edge_id), "a joint with a neighbour lost it")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A curved panel `k` (100 by 60, a parabola 20 high along x) with a
    /// bevel of 30 degrees on its side at x = 100: the bevel's corners on
    /// the front skin lie past that side by `t tan 30°`. Held only on the
    /// face itself (a reach of nothing) those corners lie where the skin
    /// was not held to the tolerance, and the plate is refused, naming the
    /// corner; held as plates are, it is built. Three scales.
    #[test]
    fn a_corner_past_where_the_skin_is_held_is_refused() {
        for k in [1e-3, 1.0, 1e6] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let rows = [0.0, 60.0];
            let profile = [(0.0, 0.0), (50.0, 40.0), (100.0, 0.0)];
            let mut ctrl = Vec::new();
            for (x, z) in profile {
                for y in rows {
                    ctrl.push(Vec3::new(x, y, z) * k);
                }
            }
            let surface = Surface::Nurbs(
                NurbsSurface::new(
                    2,
                    1,
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    vec![0.0, 0.0, 1.0, 1.0],
                    3,
                    2,
                    ctrl,
                    vec![1.0; 6],
                )
                .unwrap(),
            );
            let row = |y: f64| {
                Curve3::Nurbs(
                    NurbsCurve::polynomial(
                        2,
                        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                        profile
                            .iter()
                            .map(|(x, z)| Vec3::new(*x, y, *z) * k)
                            .collect(),
                    )
                    .unwrap(),
                )
            };
            let mut body = Body::new(BodyKind::Sheet);
            let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)]
                .map(|(x, y)| body.add_vertex(Vec3::new(x, y, 0.0) * k));
            let line = |body: &mut Body, a: usize, b: usize| {
                let (p, q) = (
                    body.vertex(corners[a]).unwrap().point,
                    body.vertex(corners[b]).unwrap().point,
                );
                body.add_edge(
                    Curve3::Line(Line::through(p, q).unwrap()),
                    (0.0, p.distance(q)),
                    corners[a],
                    corners[b],
                )
            };
            let bottom = body.add_edge(row(0.0), (0.0, 1.0), corners[0], corners[1]);
            let side = line(&mut body, 1, 2);
            let top = body.add_edge(row(60.0), (0.0, 1.0), corners[3], corners[2]);
            let start = line(&mut body, 3, 0);
            let forward = |edge| Coedge {
                edge,
                reversed: false,
            };
            let lp = body.add_loop(vec![
                forward(bottom),
                forward(side),
                Coedge {
                    edge: top,
                    reversed: true,
                },
                forward(start),
            ]);
            let face = body.add_face(surface, false, vec![lp]);
            body.add_shell(vec![face]);
            assert_eq!(check(&body, &tolerance), [], "{k}");
            let mut joints = BTreeMap::new();
            joints.insert(side, Joint::Bevel(30f64.to_radians()));
            let t = 4.0 * k;
            let refused = build(&body, &[face], t, Side::Out, &joints, 0.0, &tolerance)
                .err()
                .unwrap_or_else(|| panic!("{k}: built with the skin held on the face alone"));
            let text = refused.to_string();
            assert!(
                text.contains("the plate's corner") && text.contains("held to the tolerance"),
                "{k}: {text}"
            );
            plate(&body, &[face], t, Side::Out, &joints, &tolerance)
                .unwrap_or_else(|e| panic!("{k}: {e}"));
        }
    }

    #[test]
    fn a_circle_through_three_points_runs_their_way() {
        let a = Vec3::new(1.0, 0.0, 0.0);
        let b = Vec3::new(-1.0, 0.0, 0.0);
        let m = Vec3::new(0.0, 1.0, 0.0);
        let c = circle_through(a, b, m, false).unwrap();
        let ta = Curve3::Circle(c).project(a).unwrap();
        let tm = Curve3::Circle(c).project(m).unwrap();
        let tb = Curve3::Circle(c).project(b).unwrap();
        assert!(ta < tm && tm < tb, "{ta} {tm} {tb}");
        assert!((c.radius() - 1.0).abs() < 1e-12);
    }

    /// A bilinear patch whose row at v = 0 is `gap` long (a fan's apex
    /// when it is 0 or next to it) and whose row at v = 1 is 100 long, as
    /// a face of four lines: the reach past its vertices stays finite and
    /// at most [`MOST_REACH_FACTOR`] times the reach, at three scales and
    /// for an apex, a near-apex and a short row (the review of 2026-10-27:
    /// a speed of 1e-9 at a vertex beside an apex raised it by 1e9).
    #[test]
    fn the_reach_past_vertices_is_bounded_beside_an_apex() {
        for k in [1e-3, 1.0, 1e6] {
            for gap in [0.0, 1e-10, 1e-3, 1.0] {
                let corners = [
                    Vec3::new(0.0, 0.0, 0.0) * k,
                    Vec3::new(gap, 0.0, 0.0) * k,
                    Vec3::new(-50.0, 30.0, 5.0) * k,
                    Vec3::new(50.0, 30.0, 5.0) * k,
                ];
                let surface = Surface::Nurbs(
                    NurbsSurface::new(
                        1,
                        1,
                        vec![0.0, 0.0, 1.0, 1.0],
                        vec![0.0, 0.0, 1.0, 1.0],
                        2,
                        2,
                        vec![corners[0], corners[2], corners[1], corners[3]],
                        vec![1.0; 4],
                    )
                    .unwrap(),
                );
                let mut body = Body::new(BodyKind::Sheet);
                let ring = [0usize, 1, 3, 2];
                let mut vertices = Vec::new();
                for &i in &ring {
                    vertices.push(body.add_vertex(corners[i]));
                }
                let mut coedges = Vec::new();
                for j in 0..4 {
                    let (i0, i1) = (ring[j], ring[(j + 1) % 4]);
                    let (p, q) = (corners[i0], corners[i1]);
                    if p.distance(q) == 0.0 {
                        continue;
                    }
                    let line = Line::through(p, q).unwrap();
                    let edge = body.add_edge(
                        Curve3::Line(line),
                        (0.0, p.distance(q)),
                        vertices[j],
                        vertices[(j + 1) % 4],
                    );
                    coedges.push(Coedge {
                        edge,
                        reversed: false,
                    });
                }
                let lp = body.add_loop(coedges);
                let id = body.add_face(surface, false, vec![lp]);
                let face = body.face(id).unwrap().clone();
                let reach = 3.0 * k;
                let reaches = reach_past_vertices(&body, &face, reach, 1e-6 * k);
                for r in reaches {
                    assert!(
                        r.is_finite() && r >= reach && r <= MOST_REACH_FACTOR * reach,
                        "{k} {gap}: {reaches:?}"
                    );
                }
            }
        }
    }
}
