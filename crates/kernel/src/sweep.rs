//! Sweep: the solid a closed profile sweeps out as it is carried along a
//! path in a plane, staying at right angles to it.
//!
//! **The path** is the curves of one sketch: lines, arcs and splines that
//! join end to end into one chain, open or closed, without branches; or
//! one circle. A spline with corners (a polyline) is the pieces between
//! them. Where two curves of the chain meet they are either tangent or,
//! two straight lines only, at a corner, which is mitred: both sides end
//! on the plane that bisects the corner.
//!
//! **Kinks.** Two curves that meet a little off tangent count as tangent
//! when the profile can be turned through the kink within half the
//! tolerance, by the second-order measure of [`kink_movement`]; two lines
//! only within [`SMOOTH`], as their mitre is exact. The profile then stays
//! square to the piece before the kink until a line lays it square again:
//! a line before the kink ends square to the piece after it, a line after
//! it slides the profile as it arrives (an exact prism), an arc after it
//! carries the profile round its own axis as it arrives (see
//! [`tilted_turn`]), and a spline after it starts from it and holds
//! itself to half the tolerance of the true sweep. Kinks before arcs add
//! up until a line squares the profile, and the sum is checked at each
//! arc and at each line's end; a path that ends on an arc still turned is
//! refused unless its end cap is within half the tolerance of square
//! (`reach sin a`). A closed path whose last piece is a curve and whose
//! first is a line starts with the profile square to the last piece.
//!
//! **Where the profile sits.** The profile lies in a plane that the path
//! crosses at a right angle (within the tolerance, below): for an open path at one
//! of its ends (the one on the profile's plane; of two, the one nearer to
//! the centre of the profile), which is where the sweep starts; for a
//! closed path anywhere along it (of several crossings the one nearest to
//! the centre of the profile). The profile need not be centred on the
//! path, nor even contain it: every point of it keeps its place in the
//! frame the path carries, whose axes are the path's direction, its
//! normal in the path's plane (to the left) and the normal of that plane.
//! A profile whose plane leans away from square to the path there is laid
//! into the plane that is square, by a rigid motion (its origin to its
//! foot on that plane, its x axis to the direction of its shadow there),
//! when that moves no point of the profile by more than the linear
//! tolerance: measured exactly at the corners of a box that contains the
//! profile, which bound the movement because the motion is affine. The
//! body is then the exact sweep of the laid-in profile, and as every
//! station carries the profile rigidly, no point of it is farther than
//! that from the sweep of the profile as drawn. A larger lean (at the
//! default tolerance of 1e-6, a lean of 2e-7 radians across a profile 5
//! wide, about) is refused, naming the angle and the movement.
//!
//! **Pieces.** Along a line the profile slides (an extrusion), along an
//! arc it turns about the arc's axis (a revolution), along a spline it
//! follows frames of the spline. The body is one row of such pieces that
//! share the profile's edges where they meet:
//!
//! | Profile curve | along a line | along an arc | along a spline |
//! |---|---|---|---|
//! | line parallel to the path's plane | plane | the same plane | the same plane |
//! | line square to the path's plane | plane | cylinder | B-spline surface |
//! | any other line | plane | cone | B-spline surface |
//! | arc or circle | cylinder | torus, or a rational surface of revolution when its circle reaches the axis | rational B-spline surface |
//! | spline | B-spline surface | rational B-spline surface of revolution | B-spline surface |
//!
//! Everything along lines and arcs is exact. A line of the profile that is
//! parallel to the path's plane sweeps one plane along the whole path,
//! whatever the path does: that is one face, not one per piece (and on a
//! closed path a face with two loops). Every other curve of the profile
//! gives one face per piece of the path. A profile that is one whole
//! circle has no seam along the path when the path is lines and arcs: its
//! faces are bounded by the circles (ellipses at a mitre) between the
//! pieces alone. A circle that is the whole path is two half turns, and a
//! spline that closes on itself two pieces (cut where a profile cuts it:
//! at its inner knot nearest the middle), so that no face goes all the
//! way round and meets itself.
//!
//! **Along a spline** the swept surface is not a B-spline surface, and it
//! is approximated: the profile is placed in the frames of the spline at
//! a row of stations, and the side is the B-spline surface through them.
//! Each polynomial span of the path (the true surface is only once
//! differentiable where spans meet) is followed in one or more parts, and
//! along each part the surface is one polynomial, of degree 5, 7 or 9,
//! through the profile at the ends of the part and at the Chebyshev
//! points between them. Degrees and parts are raised ([`PLANS`]) until
//! every surface lies within half the linear tolerance of the true
//! sweep, measured at three places between each two stations in a row: a
//! point of a surface there is compared with the point of the profile at
//! the same parameter, placed in the exact frame of the path where that
//! frame's plane contains it. What was reached is [`Swept::deviation`].
//! The ends of the piece are exact, so caps and neighbouring pieces are.
//! (A cubic through equally spaced stations, which is what a loft is, has
//! a knot line per station in every face; the exact measure gave up on
//! such faces. The S of the tests, three spans, takes 43 stations at a
//! tolerance of a millionth of its size.)
//!
//! **What is refused, before anything is built:** a path that branches,
//! is in several parts or has a corner at an arc or a spline (a kink
//! more than the tolerance allows); a mitre so sharp, on a line so short,
//! that the mitres at its two ends would cross within the profile; an arc whose axis the profile reaches (the bend is
//! tighter than the profile is wide on the inside); a spline whose radius
//! of curvature comes within 2 % of how far the profile reaches towards
//! the centre of curvature (sampled at 64 places per span); a path that
//! does not start on the profile's plane or does not cross it at a right
//! angle. Everything `craft3d_topo::check` knows is refused afterwards. A
//! sweep that passes through itself because the path comes back near
//! itself is not something this module sees: the caller has to run a
//! self-intersection check (`craft3d_tess::body_self_intersections`, as
//! regeneration does).

use crate::extrude::{Origin, Role, cap_sources, each_curve_once};
use crate::loft::{Band, pinned, skin, straight};
use crate::profile::{
    CIRCLE_NOT_ALONE, Curve2, NO_SEGMENTS, ONE_SEGMENT_OPEN, Profile, Segment, Shape, SketchCurve,
    validate,
};
use crate::revolve::arcs_about;
use crate::spline2::Spline2;
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::refine::interpolation;
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, Ellipse, Line, NurbsCurve, NurbsSurface, Plane, Surface, Torus,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Most curves in the sketch of a path.
pub const MAX_PATH_CURVES: usize = 1024;

/// Most faces one sweep makes (curves of the profile times pieces of the
/// path).
pub const MAX_SWEEP_FACES: usize = 100_000;

/// The angle (in radians, as a sine) within which two lines of the path
/// count as one direction; beyond it they meet at a mitre. (Where an arc
/// or a spline meets, and whether a profile is square to the path, is a
/// matter of the linear tolerance: see the module documentation.)
pub const SMOOTH: f64 = 1e-6;

/// How a polynomial span of a spline path is followed, tried in this
/// order: in how many parts, and of what degree the surface is along
/// each (it passes through one station more than its degree per part).
const PLANS: [(usize, usize); 9] = [
    (1, 5),
    (1, 7),
    (1, 9),
    (2, 7),
    (2, 9),
    (4, 7),
    (4, 9),
    (8, 9),
    (16, 9),
];

/// How near the reach of the profile may come to the radius of curvature
/// of a spline path, as a part of that radius.
const CLEAR_OF_FOLD: f64 = 0.02;

pub(crate) type P2 = [f64; 2];

pub(crate) fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn scaled(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn length(a: P2) -> f64 {
    a[0].hypot(a[1])
}

fn distance(a: P2, b: P2) -> f64 {
    length(sub(a, b))
}

/// `v` turned a quarter turn counter-clockwise.
fn left(v: P2) -> P2 {
    [-v[1], v[0]]
}

fn unit(v: P2) -> Option<P2> {
    let l = length(v);
    (l.is_finite() && l > 0.0).then(|| scaled(v, 1.0 / l))
}

fn angle_of(v: P2) -> f64 {
    v[1].atan2(v[0])
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

pub(crate) fn refuse(curves: &[u64], reason: impl Into<String>) -> KernelError {
    let mut curves = curves.to_vec();
    curves.truncate(MAX_REPORTED);
    KernelError::Sweep {
        curves,
        reason: reason.into(),
    }
}

pub(crate) fn internal() -> KernelError {
    refuse(&[], "the sweep lost track of a curve; this is a bug")
}

fn named(ids: &[u64]) -> String {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(MAX_REPORTED);
    ids.iter()
        .map(|id| format!("e{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A swept body and where each of its faces came from: `faces[i]` belongs
/// to `FaceId(i)`.
///
/// A side is named after the curve of the profile and the curve of the
/// path it comes from, in that order; the one face of a line of the
/// profile that is parallel to the path's plane after that line alone.
/// Faces with the same sources (a spline of the profile or of the path
/// cut at its corners or where it closes, a circle that is the whole
/// path, a whole circle of the profile along a spline) are numbered by
/// [`Origin::piece`] in the order they are made: piece of the path by
/// piece, and round the profile's loop within one. The caps of an open
/// path are named after the curves of the profile's outer loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Swept {
    pub body: Body,
    pub faces: Vec<Origin>,
    /// The path closes on itself: the body is a ring without caps.
    pub closed: bool,
    /// The length of the path.
    pub length: f64,
    /// How far the surfaces along splines of the path were found to lie
    /// from the true sweep, at most; 0 for a path of lines and arcs.
    pub deviation: f64,
    /// The most stations any spline of the path was followed through; 0
    /// for a path of lines and arcs.
    pub stations: usize,
}

/// What a piece of the path runs along, in the direction of travel.
#[derive(Clone, Debug)]
enum Track {
    Line,
    /// About `center`, through `sweep`: counter-clockwise when positive.
    Arc {
        center: P2,
        sweep: f64,
    },
    /// From the first control point to the last.
    Spline(Spline2),
}

/// One piece of the path, in the sketch's own axes.
#[derive(Clone, Debug)]
struct Leg {
    source: u64,
    start: P2,
    end: P2,
    track: Track,
}

impl Leg {
    fn reversed(&self) -> Leg {
        let track = match &self.track {
            Track::Line => Track::Line,
            Track::Arc { center, sweep } => Track::Arc {
                center: *center,
                sweep: -sweep,
            },
            Track::Spline(spline) => {
                let low = spline.knots.first().copied().unwrap_or(0.0);
                let high = spline.knots.last().copied().unwrap_or(1.0);
                Track::Spline(Spline2 {
                    degree: spline.degree,
                    knots: spline.knots.iter().rev().map(|k| low + high - k).collect(),
                    control: spline.control.iter().rev().copied().collect(),
                })
            }
        };
        Leg {
            source: self.source,
            start: self.end,
            end: self.start,
            track,
        }
    }

    /// The unit direction of travel at the start, or at the end.
    fn tangent(&self, at_end: bool) -> Option<P2> {
        match &self.track {
            Track::Line => unit(sub(self.end, self.start)),
            Track::Arc { center, sweep } => {
                let at = if at_end { self.end } else { self.start };
                unit(scaled(left(sub(at, *center)), sweep.signum()))
            }
            Track::Spline(spline) => {
                let points = &spline.control;
                if at_end {
                    let last = *points.last()?;
                    points.iter().rev().find_map(|p| unit(sub(last, *p)))
                } else {
                    let first = *points.first()?;
                    points.iter().find_map(|p| unit(sub(*p, first)))
                }
            }
        }
    }

    fn is_line(&self) -> bool {
        matches!(self.track, Track::Line)
    }
}

/// The path as one chain of pieces.
struct Chain {
    legs: Vec<Leg>,
    closed: bool,
    /// A path that is one circle: its id, centre and radius. It has no
    /// legs until it is known where the profile crosses it.
    circle: Option<(u64, P2, f64)>,
}

/// Joins the curves of the path's sketch into one chain.
fn chain(curves: &[SketchCurve], tol: f64) -> Result<Chain, KernelError> {
    if curves.is_empty() {
        return Err(refuse(
            &[],
            "the sketch of the path has no curves: draw a chain of lines, arcs and splines, or a circle",
        ));
    }
    if curves.len() > MAX_PATH_CURVES {
        return Err(KernelError::TooManyCurves {
            count: curves.len(),
            limit: MAX_PATH_CURVES,
        });
    }
    let mut sorted = curves.to_vec();
    sorted.sort_by_key(|c| c.id);
    for (a, b) in sorted.iter().zip(sorted.iter().skip(1)) {
        if a.id == b.id {
            return Err(KernelError::DuplicateCurveId { id: a.id });
        }
    }
    for curve in &sorted {
        validate(curve, tol)?;
    }
    if let [only] = sorted.as_slice()
        && let Curve2::Circle { center, radius } = only.curve
    {
        return Ok(Chain {
            legs: Vec::new(),
            closed: true,
            circle: Some((only.id, center, radius)),
        });
    }
    let mut legs: Vec<Leg> = Vec::with_capacity(sorted.len());
    for curve in &sorted {
        let id = curve.id;
        match &curve.curve {
            &Curve2::Line { start, end } => legs.push(Leg {
                source: id,
                start,
                end,
                track: Track::Line,
            }),
            &Curve2::Arc { center, start, end } => {
                let sweep = wrap(angle_of(sub(end, center)) - angle_of(sub(start, center)));
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(KernelError::DegenerateCurve {
                        id,
                        reason: "the start and the end of the arc coincide; use a circle for a full turn",
                    });
                }
                legs.push(Leg {
                    source: id,
                    start,
                    end,
                    track: Track::Arc { center, sweep },
                });
            }
            Curve2::Circle { .. } => {
                return Err(refuse(
                    &[id],
                    format!(
                        "the circle e{id} is a closed path on its own; the path is one chain of curves, or one circle"
                    ),
                ));
            }
            Curve2::Spline(spline) => {
                // A spline that closes on itself is cut in two, as a
                // profile cuts it: a face that went all the way round
                // would meet itself in a seam nothing can follow.
                let closes = matches!(
                    (spline.control.first(), spline.control.last()),
                    (Some(first), Some(last)) if distance(*first, *last) <= tol
                );
                for part in spline.profile_pieces(closes) {
                    let (Some(&start), Some(&end)) = (part.control.first(), part.control.last())
                    else {
                        return Err(internal());
                    };
                    let track = if part.degree == 1 && part.control.len() == 2 {
                        if distance(start, end) <= tol {
                            return Err(KernelError::DegenerateCurve {
                                id,
                                reason: "two control points in a row of the polyline coincide",
                            });
                        }
                        Track::Line
                    } else {
                        Track::Spline(part)
                    };
                    legs.push(Leg {
                        source: id,
                        start,
                        end,
                        track,
                    });
                }
            }
        }
    }
    if legs.len() > 4 * MAX_PATH_CURVES {
        return Err(KernelError::TooManyCurves {
            count: legs.len(),
            limit: 4 * MAX_PATH_CURVES,
        });
    }

    // End `2 k` is the start of piece `k`, end `2 k + 1` its end. Ends
    // within the tolerance of each other are one place, where exactly two
    // of them meet.
    let ends: Vec<P2> = legs.iter().flat_map(|l| [l.start, l.end]).collect();
    let source_of = |end: usize| legs.get(end / 2).map_or(0, |l| l.source);
    let mut partner: Vec<Option<usize>> = Vec::with_capacity(ends.len());
    for (i, p) in ends.iter().enumerate() {
        let near: Vec<usize> = ends
            .iter()
            .enumerate()
            .filter(|(j, q)| *j != i && distance(*p, **q) <= tol)
            .map(|(j, _)| j)
            .collect();
        if near.len() > 1 {
            let mut ids: Vec<u64> = near.iter().map(|j| source_of(*j)).collect();
            ids.push(source_of(i));
            ids.sort_unstable();
            ids.dedup();
            return Err(refuse(
                &ids,
                format!(
                    "more than two curves of the path meet in one point ({}); a path is one chain without branches",
                    named(&ids)
                ),
            ));
        }
        partner.push(near.first().copied());
    }
    let free = partner.iter().position(Option::is_none);
    let mut used = vec![false; legs.len()];
    let mut ordered = Vec::with_capacity(legs.len());
    let mut entered = free.unwrap_or(0);
    for _ in 0..legs.len() {
        let index = entered / 2;
        let (Some(leg), Some(slot)) = (legs.get(index), used.get_mut(index)) else {
            return Err(internal());
        };
        if *slot {
            break;
        }
        *slot = true;
        ordered.push(if entered % 2 == 0 {
            leg.clone()
        } else {
            leg.reversed()
        });
        let leaving = entered ^ 1;
        match partner.get(leaving).copied().flatten() {
            Some(next) => entered = next,
            None => break,
        }
    }
    if ordered.len() != legs.len() {
        let ids: Vec<u64> = legs
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|(l, _)| l.source)
            .collect();
        return Err(refuse(
            &ids,
            format!(
                "the path is in more than one part: {} is not joined to the chain that starts with e{}; a path is one chain of curves that meet end to end",
                named(&ids),
                ordered.first().map_or(0, |l| l.source)
            ),
        ));
    }
    Ok(Chain {
        legs: ordered,
        closed: free.is_none(),
        circle: None,
    })
}

/// A spline of the path lifted into space with `lift`.
fn lifted(spline: &Spline2, lift: impl Fn(P2) -> Vec3) -> Result<NurbsCurve, KernelError> {
    let control = spline.control.iter().map(|p| lift(*p)).collect();
    Ok(NurbsCurve::polynomial(
        spline.degree,
        spline.knots.clone(),
        control,
    )?)
}

/// The derivative of a polynomial B-spline as a B-spline one degree down.
fn hodograph(curve: &NurbsCurve) -> Result<NurbsCurve, KernelError> {
    let (p, knots, points) = (curve.degree(), curve.knots(), curve.control_points());
    if p < 2 {
        return Err(internal());
    }
    let mut control = Vec::with_capacity(points.len().saturating_sub(1));
    for (i, (a, b)) in points.iter().zip(points.iter().skip(1)).enumerate() {
        let (Some(low), Some(high)) = (knots.get(i + 1), knots.get(i + p + 1)) else {
            return Err(internal());
        };
        let width = high - low;
        control.push(if width > 0.0 {
            (*b - *a) * (p as f64 / width)
        } else {
            Vec3::ZERO
        });
    }
    let inner = knots
        .get(1..knots.len().saturating_sub(1))
        .ok_or_else(internal)?
        .to_vec();
    Ok(NurbsCurve::polynomial(p - 1, inner, control)?)
}

/// The distinct knots of the curve's domain, ascending: where its
/// polynomial spans begin and end.
fn breaks(curve: &NurbsCurve) -> Vec<f64> {
    let (low, high) = curve.domain();
    let mut out = vec![low];
    for &knot in curve.knots() {
        if knot > low && knot < high && out.last().is_some_and(|last| knot > *last) {
            out.push(knot);
        }
    }
    out.push(high);
    out
}

/// The path with the place where the sweep starts.
struct Start {
    legs: Vec<Leg>,
    closed: bool,
    /// Where the path crosses the profile's plane, and its direction
    /// there.
    origin: P2,
    tangent: P2,
}

/// Where the path meets the profile's plane. `height` is the signed
/// distance of a point of the path's sketch from that plane, `away` its
/// distance from the centre of the profile.
fn start(
    chain: Chain,
    height: &dyn Fn(P2) -> f64,
    away: &dyn Fn(P2) -> f64,
    tol: f64,
) -> Result<Start, KernelError> {
    let no_direction = |leg: &Leg| KernelError::DegenerateCurve {
        id: leg.source,
        reason: "the curve has no direction at its end",
    };
    if !chain.closed {
        let (Some(first), Some(last)) = (chain.legs.first(), chain.legs.last()) else {
            return Err(internal());
        };
        let (d0, d1) = (height(first.start).abs(), height(last.end).abs());
        if !(d0 <= tol || d1 <= tol) {
            return Err(refuse(
                &[first.source, last.source],
                format!(
                    "the path does not start on the plane of the profile: its ends are {d0} (e{}) and {d1} (e{}) away from it; draw the profile on a plane through an end of the path, square to the path there",
                    first.source, last.source
                ),
            ));
        }
        let backwards =
            d0 > tol || (d1 <= tol && away(last.end) < away(first.start) && d1.is_finite());
        let legs: Vec<Leg> = if backwards {
            chain.legs.iter().rev().map(Leg::reversed).collect()
        } else {
            chain.legs
        };
        let first = legs.first().ok_or_else(internal)?;
        let tangent = first.tangent(false).ok_or_else(|| no_direction(first))?;
        return Ok(Start {
            origin: first.start,
            tangent,
            legs,
            closed: false,
        });
    }

    // A closed path: every place where it passes through the plane.
    let zero = height([0.0, 0.0]);
    let slope = [height([1.0, 0.0]) - zero, height([0.0, 1.0]) - zero];
    // On a circle about `center`: the angles at which it is in the plane.
    let on_circle = |center: P2, radius: f64| -> Vec<f64> {
        let reach = radius * length(slope);
        let level = -height(center) / reach;
        if !(level.is_finite() && level.abs() <= 1.0) {
            return Vec::new();
        }
        let (middle, half) = (angle_of(slope), level.acos());
        vec![middle + half, middle - half]
    };
    let rim = |center: P2, radius: f64, angle: f64| {
        add(center, [radius * angle.cos(), radius * angle.sin()])
    };
    if let Some((id, center, radius)) = chain.circle {
        let best = on_circle(center, radius)
            .into_iter()
            .map(|angle| rim(center, radius, angle))
            .min_by(|a, b| away(*a).total_cmp(&away(*b)));
        let Some(origin) = best else {
            return Err(refuse(
                &[id],
                format!(
                    "the circle e{id} does not pass through the plane of the profile; draw the profile on a plane the path crosses at a right angle"
                ),
            ));
        };
        let across = sub(scaled(center, 2.0), origin);
        let half = |start: P2, end: P2| Leg {
            source: id,
            start,
            end,
            track: Track::Arc { center, sweep: PI },
        };
        let tangent = unit(left(sub(origin, center))).ok_or_else(internal)?;
        return Ok(Start {
            legs: vec![half(origin, across), half(across, origin)],
            closed: true,
            origin,
            tangent,
        });
    }
    let mut best: Option<(usize, P2, P2, f64)> = None;
    let mut offer = |index: usize, point: P2, tangent: Option<P2>| {
        let far = away(point);
        if let Some(tangent) = tangent
            && far.is_finite()
            && best.is_none_or(|(_, _, _, least)| far < least)
        {
            best = Some((index, point, tangent, far));
        }
    };
    for (index, leg) in chain.legs.iter().enumerate() {
        match &leg.track {
            Track::Line => {
                let (a, b) = (height(leg.start), height(leg.end));
                let slack = tol / distance(leg.start, leg.end);
                let f = a / (a - b);
                if f.is_finite() && f >= -slack && f <= 1.0 + slack {
                    let f = f.clamp(0.0, 1.0);
                    offer(
                        index,
                        add(leg.start, scaled(sub(leg.end, leg.start), f)),
                        leg.tangent(false),
                    );
                }
            }
            &Track::Arc { center, sweep } => {
                let radius = distance(leg.start, center);
                let begin = angle_of(sub(leg.start, center));
                for angle in on_circle(center, radius) {
                    let turned = wrap((angle - begin) * sweep.signum());
                    let slack = tol / radius;
                    if turned <= sweep.abs() + slack || turned >= TAU - slack {
                        let at = rim(center, radius, angle);
                        offer(
                            index,
                            at,
                            unit(scaled(left(sub(at, center)), sweep.signum())),
                        );
                    }
                }
            }
            Track::Spline(spline) => {
                let curve = lifted(spline, |p| Vec3::new(p[0], p[1], 0.0))?;
                let at = |t: f64| {
                    let p = curve.eval(t);
                    height([p.x, p.y])
                };
                let knots = breaks(&curve);
                for (&low, &high) in knots.iter().zip(knots.iter().skip(1)) {
                    const STEPS: u32 = 32;
                    for step in 0..STEPS {
                        let mut a = low + (high - low) * f64::from(step) / f64::from(STEPS);
                        let mut b = low + (high - low) * f64::from(step + 1) / f64::from(STEPS);
                        let (ha, hb) = (at(a), at(b));
                        if !(ha.is_finite() && hb.is_finite()) || (ha > 0.0) == (hb > 0.0) {
                            continue;
                        }
                        for _ in 0..60 {
                            let middle = (a + b) / 2.0;
                            if (at(middle) > 0.0) == (ha > 0.0) {
                                a = middle;
                            } else {
                                b = middle;
                            }
                        }
                        let (p, d) = curve.derivatives((a + b) / 2.0);
                        offer(index, [p.x, p.y], unit([d.x, d.y]));
                    }
                }
            }
        }
    }
    let Some((index, origin, tangent, _)) = best else {
        let ids: Vec<u64> = chain.legs.iter().map(|l| l.source).collect();
        return Err(refuse(
            &ids,
            "the closed path does not pass through the plane of the profile; draw the profile on a plane the path crosses at a right angle",
        ));
    };
    // The body is made of whole pieces of the path: it starts where the
    // piece the profile is on ends.
    let mut legs = chain.legs;
    let turn = (index + 1) % legs.len().max(1);
    legs.rotate_left(turn);
    Ok(Start {
        legs,
        closed: true,
        origin,
        tangent,
    })
}

/// What a curve of the profile is, in the axes the path carries: `a` to
/// the left of the path in its plane, `b` along the normal of that plane.
#[derive(Clone, Debug)]
pub(crate) enum Form {
    Line,
    /// About `center` from the angle `from` through `sweep`:
    /// counter-clockwise when positive.
    Arc {
        center: P2,
        radius: f64,
        from: f64,
        sweep: f64,
    },
    /// A whole circle from its point at the angle `from`.
    Circle {
        center: P2,
        radius: f64,
        from: f64,
        counter_clockwise: bool,
    },
    /// The spline in its own direction, which the loop runs against when
    /// `against`.
    Spline {
        own: Spline2,
        against: bool,
    },
}

/// One curve of the profile's loop, in the direction the loop is walked.
#[derive(Clone, Debug)]
pub(crate) struct Element {
    pub(crate) source: u64,
    pub(crate) start: P2,
    pub(crate) end: P2,
    pub(crate) form: Form,
    /// A line parallel to the path's plane: it sweeps one plane.
    pub(crate) flat: bool,
}

impl Element {
    /// True when the loop runs against the direction of the element's
    /// edges: an arc's are counter-clockwise, a spline's its own.
    pub(crate) fn against(&self) -> bool {
        match &self.form {
            Form::Line => false,
            Form::Arc { sweep, .. } => *sweep < 0.0,
            Form::Circle {
                counter_clockwise, ..
            } => !counter_clockwise,
            Form::Spline { against, .. } => *against,
        }
    }

    pub(crate) fn whole(&self) -> bool {
        matches!(self.form, Form::Circle { .. })
    }

    /// The same curve walked the other way.
    pub(crate) fn flipped(&self) -> Element {
        let form = match &self.form {
            Form::Line => Form::Line,
            &Form::Arc {
                center,
                radius,
                from,
                sweep,
            } => Form::Arc {
                center,
                radius,
                from: from + sweep,
                sweep: -sweep,
            },
            &Form::Circle {
                center,
                radius,
                from,
                counter_clockwise,
            } => Form::Circle {
                center,
                radius,
                from,
                counter_clockwise: !counter_clockwise,
            },
            Form::Spline { own, against } => Form::Spline {
                own: own.clone(),
                against: !against,
            },
        };
        Element {
            source: self.source,
            start: self.end,
            end: self.start,
            form,
            flat: self.flat,
        }
    }

    /// The least and the greatest `a` the element reaches.
    fn reach(&self) -> (f64, f64) {
        let (mut low, mut high) = (
            self.start[0].min(self.end[0]),
            self.start[0].max(self.end[0]),
        );
        match &self.form {
            Form::Line => {}
            &Form::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                // The angle 0 is the far right of its circle, π the far left.
                let (begin, span) = if sweep >= 0.0 {
                    (from, sweep)
                } else {
                    (from + sweep, -sweep)
                };
                if wrap(-begin) <= span {
                    high = high.max(center[0] + radius);
                }
                if wrap(PI - begin) <= span {
                    low = low.min(center[0] - radius);
                }
            }
            &Form::Circle { center, radius, .. } => {
                (low, high) = (center[0] - radius, center[0] + radius);
            }
            Form::Spline { own, .. } => {
                let (least, most) = own.extent([0.0, 0.0], [1.0, 0.0]);
                (low, high) = (low.min(least), high.max(most));
            }
        }
        (low, high)
    }

    /// A point of a line, an arc or a circle away from its ends, and the
    /// direction the loop runs in there.
    pub(crate) fn middle(&self) -> (P2, P2) {
        let on_circle = |center: P2, radius: f64, angle: f64, forward: bool| {
            let (s, c) = angle.sin_cos();
            let along = if forward { [-s, c] } else { [s, -c] };
            (add(center, [radius * c, radius * s]), along)
        };
        match &self.form {
            Form::Line | Form::Spline { .. } => (
                scaled(add(self.start, self.end), 0.5),
                sub(self.end, self.start),
            ),
            &Form::Arc {
                center,
                radius,
                from,
                sweep,
            } => on_circle(center, radius, from + sweep / 2.0, sweep >= 0.0),
            &Form::Circle {
                center,
                radius,
                from,
                counter_clockwise,
            } => on_circle(center, radius, from + 1.0, counter_clockwise),
        }
    }

    /// The range of angles of an arc or a circle in the direction of its
    /// edge (counter-clockwise): where it begins, in `[0, 2π)`, and how
    /// far it goes.
    fn angles(&self) -> Option<(f64, f64)> {
        match self.form {
            Form::Arc { from, sweep, .. } if sweep >= 0.0 => Some((wrap(from), sweep)),
            Form::Arc { from, sweep, .. } => Some((wrap(from + sweep), -sweep)),
            Form::Circle { from, .. } => Some((wrap(from), TAU)),
            Form::Line | Form::Spline { .. } => None,
        }
    }
}

/// The loops of `profile`, the outer one first, as elements laid by `map`
/// into the plane the path carries. Seen along the path the outer loop
/// runs counter-clockwise and the holes clockwise, whichever way the
/// profile's plane faces. A loop without segments, a loop of one segment
/// that is not a circle, and a circle in a loop with other segments are
/// refused.
pub(crate) fn profile_loops(
    profile: &Profile,
    map: &Map2,
    tol: f64,
) -> Result<Vec<Vec<Element>>, KernelError> {
    let mut loops: Vec<Vec<Element>> = Vec::with_capacity(1 + profile.holes.len());
    for path in std::iter::once(&profile.outer).chain(&profile.holes) {
        let mut elements = path
            .segments
            .iter()
            .map(|s| map.element(s, tol))
            .collect::<Result<Vec<_>, _>>()?;
        match elements.as_slice() {
            [] => {
                return Err(KernelError::InvalidProfile {
                    reason: NO_SEGMENTS,
                });
            }
            [only] if !only.whole() => {
                return Err(KernelError::InvalidProfile {
                    reason: ONE_SEGMENT_OPEN,
                });
            }
            [_] => {}
            several if several.iter().any(Element::whole) => {
                return Err(KernelError::InvalidProfile {
                    reason: CIRCLE_NOT_ALONE,
                });
            }
            _ => {}
        }
        if map.mirrored {
            elements = elements.iter().rev().map(Element::flipped).collect();
        }
        loops.push(elements);
    }
    Ok(loops)
}

/// A rigid motion of the plane, or one with a reflection: from the
/// profile's sketch to the axes the path carries.
pub(crate) struct Map2 {
    pub(crate) origin: P2,
    pub(crate) x: P2,
    pub(crate) y: P2,
    /// The motion turns the plane over.
    pub(crate) mirrored: bool,
}

impl Map2 {
    fn at(&self, p: P2) -> P2 {
        add(self.origin, add(scaled(self.x, p[0]), scaled(self.y, p[1])))
    }

    pub(crate) fn element(&self, segment: &Segment, tol: f64) -> Result<Element, KernelError> {
        let (mut start, mut end) = (self.at(segment.start), self.at(segment.end));
        if !(start.iter().chain(&end).all(|v| v.is_finite())) {
            return Err(KernelError::InvalidProfile {
                reason: "a point of the profile is not finite",
            });
        }
        let positive = |radius: f64| {
            if radius.is_finite() && radius > 0.0 {
                Ok(radius)
            } else {
                Err(KernelError::InvalidSize {
                    name: "radius",
                    value: radius,
                })
            }
        };
        let form = match &segment.shape {
            Shape::Line => Form::Line,
            &Shape::Arc { center, radius } => {
                let (_, sweep) = segment.angles().ok_or(KernelError::InvalidProfile {
                    reason: "an arc without angles",
                })?;
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(KernelError::InvalidProfile {
                        reason: "an arc whose start and end coincide; a full turn is a circle",
                    });
                }
                let center = self.at(center);
                let clockwise = segment.reversed != self.mirrored;
                Form::Arc {
                    center,
                    radius: positive(radius)?,
                    from: angle_of(sub(start, center)),
                    sweep: if clockwise { -sweep } else { sweep },
                }
            }
            &Shape::Circle { center, radius } => {
                // From its point at angle 0 of the sketch, whatever the
                // segment says.
                start = self.at(add(center, [radius, 0.0]));
                end = start;
                let center = self.at(center);
                Form::Circle {
                    center,
                    radius: positive(radius)?,
                    from: angle_of(sub(start, center)),
                    counter_clockwise: segment.reversed == self.mirrored,
                }
            }
            Shape::Spline(spline) => {
                if let Some(reason) = spline.fault() {
                    return Err(KernelError::DegenerateCurve {
                        id: segment.source,
                        reason,
                    });
                }
                Form::Spline {
                    own: Spline2 {
                        degree: spline.degree,
                        knots: spline.knots.clone(),
                        control: spline.control.iter().map(|p| self.at(*p)).collect(),
                    },
                    against: segment.reversed,
                }
            }
        };
        let flat = matches!(form, Form::Line) && (start[1] - end[1]).abs() <= tol / 8.0;
        Ok(Element {
            source: segment.source,
            start,
            end,
            form,
            flat,
        })
    }
}

/// A whole circle as four quarter arcs from where it starts.
pub(crate) fn quarters(element: &Element) -> Vec<Element> {
    let &Form::Circle {
        center,
        radius,
        from,
        counter_clockwise,
    } = &element.form
    else {
        return vec![element.clone()];
    };
    let step = if counter_clockwise {
        FRAC_PI_2
    } else {
        -FRAC_PI_2
    };
    let at = |k: u8| {
        if k == 0 || k == 4 {
            element.start
        } else {
            let (s, c) = (from + step * f64::from(k)).sin_cos();
            add(center, [radius * c, radius * s])
        }
    };
    (0..4_u8)
        .map(|k| Element {
            source: element.source,
            start: at(k),
            end: at(k + 1),
            form: Form::Arc {
                center,
                radius,
                from: from + step * f64::from(k),
                sweep: step,
            },
            flat: false,
        })
        .collect()
}

/// Where the profile is at one place of the path: the point `(a, b)` of
/// it is at `origin + a u + b b`. Square to the path `u` is a unit vector;
/// at a mitre it is longer and leans along the path.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Station {
    pub(crate) origin: Vec3,
    pub(crate) u: Vec3,
    pub(crate) b: Vec3,
    pub(crate) square: bool,
}

impl Station {
    pub(crate) fn at(&self, p: P2) -> Vec3 {
        self.origin + self.u * p[0] + self.b * p[1]
    }

    pub(crate) fn vector(&self, d: P2) -> Vec3 {
        self.u * d[0] + self.b * d[1]
    }

    /// The unit normal of the station's plane, the way the path goes.
    pub(crate) fn forward(&self) -> Result<Vec3, KernelError> {
        self.u.cross(self.b).normalized().ok_or_else(internal)
    }

    /// The coordinates of a point of the station's plane.
    pub(crate) fn local(&self, p: Vec3) -> P2 {
        let d = p - self.origin;
        [d.dot(self.u) / self.u.length_squared(), d.dot(self.b)]
    }
}

/// The control points, weights and knots of the arc about `center` from
/// the angle `begin` counter-clockwise through `span`, as a rational
/// quadratic in pieces of at most a quarter turn (*The NURBS Book* §7.5).
fn arc_net(center: P2, radius: f64, begin: f64, span: f64) -> (Vec<P2>, Vec<f64>, Vec<f64>) {
    let pieces = ((span / FRAC_PI_2) - 1e-9).ceil().clamp(1.0, 4.0);
    let step = span / pieces;
    let pieces = pieces as usize;
    let weight = (step / 2.0).cos();
    let at = |angle: f64, reach: f64| {
        let (s, c) = angle.sin_cos();
        add(center, [reach * c, reach * s])
    };
    let mut control = vec![at(begin, radius)];
    let mut weights = vec![1.0];
    let mut knots = vec![0.0; 3];
    for i in 0..pieces {
        let from = begin + step * i as f64;
        control.push(at(from + step / 2.0, radius / weight));
        weights.push(weight);
        control.push(at(from + step, radius));
        weights.push(1.0);
        if i + 1 < pieces {
            let knot = (i + 1) as f64 / pieces as f64;
            knots.extend([knot, knot]);
        }
    }
    knots.extend([1.0; 3]);
    (control, weights, knots)
}

/// The element at a station as a spline, in the direction of its edge,
/// from `ends.0` to `ends.1` (the vertices, to the bit).
pub(crate) fn curve_at(
    element: &Element,
    station: &Station,
    ends: (Vec3, Vec3),
) -> Result<NurbsCurve, KernelError> {
    let curve = match &element.form {
        Form::Line => NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![ends.0, ends.1])?,
        &Form::Arc { center, radius, .. } | &Form::Circle { center, radius, .. } => {
            let (begin, span) = element.angles().ok_or_else(internal)?;
            let (control, weights, knots) = arc_net(center, radius, begin, span);
            let control = control.iter().map(|p| station.at(*p)).collect();
            NurbsCurve::new(2, knots, control, weights)?
        }
        Form::Spline { own, .. } => NurbsCurve::polynomial(
            own.degree,
            own.knots.clone(),
            own.control.iter().map(|p| station.at(*p)).collect(),
        )?,
    };
    pinned(&curve, ends.0, ends.1)
}

/// The ends of an element at a station, in the direction of its edge.
pub(crate) fn edge_ends(element: &Element, station: &Station) -> (Vec3, Vec3) {
    let (start, end) = (station.at(element.start), station.at(element.end));
    if element.against() {
        (end, start)
    } else {
        (start, end)
    }
}

/// How the profile gets from one station to the next.
pub(crate) enum Motion {
    /// Straight along a unit direction.
    Slide { along: Vec3 },
    /// About the z axis of `frame`, counter-clockwise through `angle`,
    /// from its xz plane.
    Turn { frame: Frame, angle: f64 },
    /// Through the stations (the first and the last are the ends), joined
    /// by `band`.
    Follow { stations: Vec<Station>, band: Band },
}

/// `v` turned about the z axis of `frame` through `angle`.
fn turned(frame: &Frame, v: Vec3, angle: f64) -> Vec3 {
    let (x, y, z) = (v.dot(frame.x()), v.dot(frame.y()), v.dot(frame.z()));
    let (s, c) = angle.sin_cos();
    frame.vector(x * c - y * s, x * s + y * c, z)
}

/// The surface a spline in the xz plane of `frame` sweeps as it turns
/// about the z axis through `angle`: u around the axis, v along the
/// curve.
fn revolved(curve: &NurbsCurve, frame: &Frame, angle: f64) -> Result<NurbsSurface, KernelError> {
    let ring = arcs_about(0.0, angle);
    let count = curve.control_points().len();
    let mut control = Vec::with_capacity(ring.points.len() * count);
    let mut weights = Vec::with_capacity(ring.points.len() * count);
    for &(x, y, weight) in &ring.points {
        for (p, own) in curve.control_points().iter().zip(curve.weights()) {
            let (rho, _, z) = frame.local(*p);
            control.push(frame.point(rho * x, rho * y, z));
            weights.push(weight * own);
        }
    }
    Ok(NurbsSurface::new(
        2,
        curve.degree(),
        ring.knots,
        curve.knots().to_vec(),
        ring.points.len(),
        count,
        control,
        weights,
    )?)
}

/// True when the outside of a face, which is towards `outward` at its
/// point `at`, is against the normal of its surface.
pub(crate) fn against_normal(
    surface: &Surface,
    at: Vec3,
    outward: Vec3,
) -> Result<bool, KernelError> {
    let (u, v) = surface.project(at)?;
    let normal = surface.normal(u, v).ok_or_else(internal)?;
    let along = normal.dot(outward);
    if !(along.is_finite() && along.abs() > 1e-9 * outward.length()) {
        return Err(internal());
    }
    Ok(along < 0.0)
}

/// The B-spline surface, or the plane it lies in when it lies in one; an
/// error when its normal vanishes somewhere.
pub(crate) fn flat_or(
    surface: NurbsSurface,
    tol: f64,
    profile: u64,
    path: u64,
) -> Result<Surface, KernelError> {
    const GRID: u32 = 8;
    let ((u0, u1), (v0, v1)) = surface.domain();
    let mut middle = None;
    for i in 0..=GRID {
        for j in 0..=GRID {
            let (u, v) = (
                u0 + (u1 - u0) * f64::from(i) / f64::from(GRID),
                v0 + (v1 - v0) * f64::from(j) / f64::from(GRID),
            );
            let (point, du, dv) = surface.derivatives(u, v);
            let normal = du.cross(dv);
            let size = normal.length();
            if !size.is_finite() || size <= 1e-9 * du.length() * dv.length() {
                return Err(refuse(
                    &[path],
                    format!(
                        "the face e{profile} of the profile sweeps along e{path} folds onto itself"
                    ),
                ));
            }
            if i == GRID / 2 && j == GRID / 2 {
                middle = Some((point, normal, du));
            }
        }
    }
    let (origin, normal, along) = middle.ok_or_else(internal)?;
    let normal = normal.normalized().ok_or_else(internal)?;
    let flat = surface
        .control_points()
        .iter()
        .all(|p| (*p - origin).dot(normal).abs() <= tol / 8.0);
    if flat && let Some(frame) = Frame::new(origin, normal, along) {
        return Ok(Surface::Plane(Plane::new(frame)));
    }
    Ok(Surface::Nurbs(surface))
}

/// Where the stations of one part of the path are, as parts of it from 0
/// to 1: the ends and the Chebyshev points between them, through which a
/// polynomial of degree `degree` follows a smooth curve about as well as
/// one of that degree can.
pub(crate) fn stations_of(degree: usize) -> Vec<f64> {
    (0..=degree)
        .map(|j| {
            if j == 0 {
                0.0
            } else if j == degree {
                1.0
            } else {
                0.5 * (1.0 - (PI * j as f64 / degree as f64).cos())
            }
        })
        .collect()
}

/// The band that joins the stations of `parts` parts of a path, each
/// followed by one polynomial of degree `degree` through its stations at
/// `at` (see [`stations_of`]); the parts are joined where they meet.
pub(crate) fn part_band(parts: usize, degree: usize, at: &[f64]) -> Result<Band, KernelError> {
    let count = parts
        .checked_mul(degree)
        .and_then(|n| n.checked_add(1))
        .filter(|n| *n <= MAX_SWEEP_FACES)
        .ok_or_else(internal)?;
    let (_, matrix) = interpolation(at, degree)?;
    let unit = |k: usize| -> Vec<f64> { (0..count).map(|i| f64::from(u8::from(i == k))).collect() };
    let mut knots = vec![0.0; degree + 1];
    let mut mix = Vec::with_capacity(count);
    for part in 0..parts {
        let base = part * degree;
        knots.extend(std::iter::repeat_n(
            (part + 1) as f64,
            if part + 1 < parts { degree } else { degree + 1 },
        ));
        for (r, row) in matrix.iter().enumerate() {
            if r == 0 && part > 0 {
                continue;
            }
            // The band passes through the ends of each part exactly.
            mix.push(if r == 0 {
                unit(base)
            } else if r == degree {
                unit(base + degree)
            } else {
                let mut wide = vec![0.0; count];
                for (slot, value) in wide.iter_mut().skip(base).zip(row) {
                    *slot = *value;
                }
                wide
            });
        }
    }
    Ok(Band {
        from: 0,
        to: count - 1,
        degree,
        knots,
        mix,
    })
}

/// A spline of the path in space, and what is needed to follow it.
struct Spine {
    curve: NurbsCurve,
    velocity: NurbsCurve,
    b: Vec3,
}

impl Spine {
    /// The frame of the path at parameter `t`.
    fn station(&self, t: f64) -> Result<Station, KernelError> {
        let (origin, d) = self.curve.derivatives(t);
        let along = d.normalized().ok_or_else(internal)?;
        let u = self.b.cross(along).normalized().ok_or_else(internal)?;
        Ok(Station {
            origin,
            u,
            b: self.b,
            square: true,
        })
    }

    /// The parameter, near `guess`, of the station whose plane holds `p`.
    fn foot(&self, p: Vec3, guess: f64) -> f64 {
        let (low, high) = self.curve.domain();
        let mut t = guess;
        for _ in 0..24 {
            let (c, d) = self.curve.derivatives(t);
            let (_, dd) = self.velocity.derivatives(t);
            let slope = (p - c).dot(dd) - d.length_squared();
            let step = (p - c).dot(d) / slope;
            if !step.is_finite() {
                break;
            }
            t = (t - step).clamp(low, high);
            if step.abs() <= 1e-15 * (high - low) {
                break;
            }
        }
        t
    }
}

/// What following a spline of the path gave.
struct Followed {
    stations: Vec<Station>,
    band: Band,
    deviation: f64,
    /// How many stations there are.
    count: usize,
    length: f64,
}

/// The stations along the spline of leg `source`, from the station
/// `first` the profile arrives in, and the band through them: as many
/// per span as it takes to bring every surface within half the tolerance
/// of the true sweep.
fn follow(
    spline: &Spline2,
    source: u64,
    first: &Station,
    plane: &Frame,
    loops: &[Vec<Element>],
    reach: (f64, f64),
    tol: f64,
) -> Result<Followed, KernelError> {
    // The spline as drawn, moved by whatever the pieces before it left
    // the profile off the drawn path (rounding).
    let begin = spline.control.first().ok_or_else(internal)?;
    let offset = first.origin - plane.point(begin[0], begin[1], 0.0);
    let curve = lifted(spline, |p| plane.point(p[0], p[1], 0.0) + offset)?;
    let spine = Spine {
        velocity: hodograph(&curve)?,
        curve,
        b: first.b,
    };
    let knots = breaks(&spine.curve);
    let spans = knots.len().saturating_sub(1);
    if spans == 0 {
        return Err(internal());
    }

    // Where the path bends tighter than the profile reaches towards the
    // centre of the bend, the sweep folds over itself. Its length on the
    // way.
    const SAMPLES: u32 = 64;
    let mut length = 0.0;
    for (&low, &high) in knots.iter().zip(knots.iter().skip(1)) {
        for i in 0..=SAMPLES {
            let t = low + (high - low) * f64::from(i) / f64::from(SAMPLES);
            let (at, d) = spine.curve.derivatives(t);
            let (_, dd) = spine.velocity.derivatives(t);
            let speed = d.length();
            // Simpson's rule over the span.
            let weight = if i == 0 || i == SAMPLES {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            length += weight * speed * (high - low) / (3.0 * f64::from(SAMPLES));
            let curvature = d.cross(dd).dot(spine.b) / (speed * speed * speed);
            if !(curvature.is_finite() && speed > 0.0) {
                return Err(refuse(
                    &[source],
                    format!(
                        "the spline e{source} of the path stops or turns on the spot (a cusp); a profile cannot be carried through it"
                    ),
                ));
            }
            // To the left of the path when it turns left, and the other
            // way round.
            let inside = if curvature > 0.0 { reach.1 } else { -reach.0 };
            if curvature.abs() * inside > 1.0 - CLEAR_OF_FOLD {
                let (x, y, _) = plane.local(at - offset);
                return Err(refuse(
                    &[source],
                    format!(
                        "the path bends too tightly for the profile at ({x}, {y}) on e{source}: its radius of curvature there is {}, and the profile reaches {inside} towards the centre of the bend; the sweep would fold over itself",
                        1.0 / curvature.abs()
                    ),
                ));
            }
        }
    }

    let mut reached = f64::INFINITY;
    let mut most = 0;
    for (parts, degree) in PLANS {
        let within = stations_of(degree);
        let mut parameters = Vec::with_capacity(spans * parts * degree + 1);
        let mut stations = Vec::with_capacity(spans * parts * degree + 1);
        for (&low, &high) in knots.iter().zip(knots.iter().skip(1)) {
            for part in 0..parts {
                let from = low + (high - low) * part as f64 / parts as f64;
                let to = if part + 1 == parts {
                    high
                } else {
                    low + (high - low) * (part + 1) as f64 / parts as f64
                };
                for (step, share) in within.iter().enumerate() {
                    if step == 0 && !stations.is_empty() {
                        continue;
                    }
                    let t = if step == degree {
                        to
                    } else {
                        from + (to - from) * share
                    };
                    parameters.push(t);
                    stations.push(if stations.is_empty() {
                        *first
                    } else {
                        spine.station(t)?
                    });
                }
            }
        }
        most = stations.len();
        let band = part_band(spans * parts, degree, &within)?;
        let mut worst: f64 = 0.0;
        for element in loops.iter().flatten() {
            let column = stations
                .iter()
                .map(|s| curve_at(element, s, edge_ends(element, s)))
                .collect::<Result<Vec<_>, _>>()?;
            let surface = skin(&column, &band)?;
            let own = column.first().ok_or_else(internal)?;
            let ((u0, u1), _) = surface.domain();
            const ACROSS: u32 = 8;
            // Between each two stations in a row, at three places.
            for (gap, (&t0, &t1)) in parameters.iter().zip(parameters.iter().skip(1)).enumerate() {
                let (part, step) = (gap / degree, gap % degree);
                let (Some(&s0), Some(&s1)) = (within.get(step), within.get(step + 1)) else {
                    return Err(internal());
                };
                for share in [0.25, 0.5, 0.75] {
                    let v = part as f64 + s0 + (s1 - s0) * share;
                    for i in 0..=ACROSS {
                        let u = u0 + (u1 - u0) * f64::from(i) / f64::from(ACROSS);
                        let made = surface.eval(u, v);
                        let t = spine.foot(made, t0 + (t1 - t0) * share);
                        let found = spine.station(t)?.local(made);
                        let meant = first.local(own.eval(u));
                        worst = worst.max(distance(found, meant));
                    }
                }
            }
        }
        if !worst.is_finite() {
            return Err(internal());
        }
        reached = worst;
        if worst <= tol / 2.0 {
            return Ok(Followed {
                stations,
                band,
                deviation: worst,
                count: most,
                length,
            });
        }
    }
    Err(refuse(
        &[source],
        format!(
            "the sweep along the spline e{source} cannot be brought within the tolerance: with {most} stations along the spline its surfaces are still {reached} from the true sweep, and half the tolerance ({}) is allowed; give the document a coarser tolerance, or draw the path with lines and arcs",
            tol / 2.0
        ),
    ))
}

/// The vertices and edges of the profile at one station: per loop, per
/// element, the vertex it starts at and its edge with whether the loop
/// runs against it. A line that sweeps one plane has an edge only at the
/// ends of an open path.
#[derive(Clone)]
pub(crate) struct Ring {
    pub(crate) vertices: Vec<Vec<VertexId>>,
    pub(crate) edges: Vec<Vec<Option<(EdgeId, bool)>>>,
}

pub(crate) struct Builder {
    pub(crate) body: Body,
    pub(crate) faces: Vec<Origin>,
    pub(crate) tol: f64,
}

pub(crate) fn coedge(edge: EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

impl Builder {
    pub(crate) fn point(&self, vertex: VertexId) -> Result<Vec3, KernelError> {
        self.body
            .vertex(vertex)
            .map(|v| v.point)
            .ok_or_else(internal)
    }

    pub(crate) fn face(
        &mut self,
        surface: Surface,
        reversed: bool,
        loops: Vec<Vec<Coedge>>,
        role: Role,
        sources: Vec<u64>,
    ) -> FaceId {
        let loops = loops.into_iter().map(|l| self.body.add_loop(l)).collect();
        self.faces.push(Origin {
            role,
            sources,
            piece: 0,
        });
        self.body.add_face(surface, reversed, loops)
    }

    pub(crate) fn ring(
        &mut self,
        loops: &[Vec<Element>],
        station: &Station,
        every_edge: bool,
    ) -> Result<Ring, KernelError> {
        let mut ring = Ring {
            vertices: Vec::with_capacity(loops.len()),
            edges: Vec::with_capacity(loops.len()),
        };
        for elements in loops {
            let vertices: Vec<VertexId> = elements
                .iter()
                .map(|e| self.body.add_vertex(station.at(e.start)))
                .collect();
            let after = vertices.iter().cycle().skip(1);
            let mut edges = Vec::with_capacity(elements.len());
            for (element, (&here, &there)) in elements.iter().zip(vertices.iter().zip(after)) {
                edges.push(if element.flat && !every_edge {
                    None
                } else {
                    Some(self.ring_edge(element, station, here, there)?)
                });
            }
            ring.vertices.push(vertices);
            ring.edges.push(edges);
        }
        Ok(ring)
    }

    /// The edge of `element` at `station` between the vertices the loop
    /// passes it from and to, and whether the loop runs against it.
    fn ring_edge(
        &mut self,
        element: &Element,
        station: &Station,
        here: VertexId,
        there: VertexId,
    ) -> Result<(EdgeId, bool), KernelError> {
        let against = element.against();
        let (first, last) = if against {
            (there, here)
        } else {
            (here, there)
        };
        let (curve, range) = match &element.form {
            Form::Line => {
                let (p, q) = (self.point(first)?, self.point(last)?);
                (Curve3::Line(Line::through(p, q)?), (0.0, p.distance(q)))
            }
            &Form::Arc { center, radius, .. } | &Form::Circle { center, radius, .. } => {
                let (begin, span) = element.angles().ok_or_else(internal)?;
                let frame = Frame::new(station.at(center), station.forward()?, station.u)
                    .ok_or_else(internal)?;
                let curve = if station.square {
                    Curve3::Circle(Circle::new(frame, radius)?)
                } else {
                    Curve3::Ellipse(Ellipse::new(frame, radius * station.u.length(), radius)?)
                };
                (curve, (begin, begin + span))
            }
            Form::Spline { .. } => {
                let curve = curve_at(element, station, (self.point(first)?, self.point(last)?))?;
                let range = curve.domain();
                (Curve3::Nurbs(curve), range)
            }
        };
        Ok((self.body.add_edge(curve, range, first, last), against))
    }

    /// The edge a vertex of the profile sweeps between two stations, and
    /// its length.
    pub(crate) fn rail(
        &mut self,
        motion: &Motion,
        at: P2,
        from: VertexId,
        to: VertexId,
    ) -> Result<(EdgeId, f64), KernelError> {
        let (p, q) = (self.point(from)?, self.point(to)?);
        let (curve, range, length) = match motion {
            Motion::Slide { .. } => {
                let length = p.distance(q);
                (Curve3::Line(Line::through(p, q)?), (0.0, length), length)
            }
            Motion::Turn { frame, angle } => {
                // The circle starts at the vertex, which is off the
                // frame's x axis when the profile arrives turned.
                let (x, y, z) = frame.local(p);
                let rho = x.hypot(y);
                let level = if y == 0.0 {
                    frame.at(frame.point(0.0, 0.0, z))
                } else {
                    let centre = frame.point(0.0, 0.0, z);
                    Frame::new(centre, frame.z(), p - centre)
                }
                .ok_or_else(internal)?;
                (
                    Curve3::Circle(Circle::new(level, rho)?),
                    (0.0, *angle),
                    rho * angle,
                )
            }
            Motion::Follow { stations, band } => {
                let through: Vec<Vec3> = stations.iter().map(|s| s.at(at)).collect();
                let length = through
                    .iter()
                    .zip(through.iter().skip(1))
                    .map(|(a, b)| a.distance(*b))
                    .sum();
                let mut control = band.points(&through);
                if let Some(first) = control.first_mut() {
                    *first = p;
                }
                if let Some(last) = control.last_mut() {
                    *last = q;
                }
                match straight(&control, self.tol / 8.0) {
                    Some((line, range)) => (line, range, length),
                    None => {
                        let curve =
                            NurbsCurve::polynomial(band.degree, band.knots.clone(), control)?;
                        let range = curve.domain();
                        (Curve3::Nurbs(curve), range, length)
                    }
                }
            }
        };
        Ok((self.body.add_edge(curve, range, from, to), length))
    }

    /// The surface `element` sweeps from `station` by `motion`, and
    /// whether the outside is against its normal.
    pub(crate) fn surface(
        &self,
        motion: &Motion,
        element: &Element,
        station: &Station,
        next: &Station,
        path: u64,
    ) -> Result<(Surface, bool), KernelError> {
        let against = element.against();
        let curve = |s: &Station| curve_at(element, s, edge_ends(element, s));
        match motion {
            Motion::Slide { along } => match &element.form {
                Form::Line => {
                    let (p, q) = (station.at(element.start), station.at(element.end));
                    let frame = Frame::new(p, (q - p).cross(*along), q - p).ok_or_else(internal)?;
                    Ok((Surface::Plane(Plane::new(frame)), false))
                }
                &Form::Arc { center, radius, .. } | &Form::Circle { center, radius, .. } => {
                    let frame =
                        Frame::new(station.at(center), *along, station.u).ok_or_else(internal)?;
                    Ok((Surface::Cylinder(Cylinder::new(frame, radius)?), against))
                }
                Form::Spline { .. } => {
                    let column = [curve(station)?, curve(next)?];
                    Ok((
                        Surface::Nurbs(skin(&column, &Band::straight(0, 1, 2))?),
                        against,
                    ))
                }
            },
            Motion::Turn { frame, angle } => {
                let place = |p: P2| {
                    let (x, y, z) = frame.local(station.at(p));
                    (x.hypot(y), z)
                };
                let level = |z: f64| frame.at(frame.point(0.0, 0.0, z)).ok_or_else(internal);
                let analytic = match &element.form {
                    Form::Line => {
                        let ((r0, z0), (r1, z1)) = (place(element.start), place(element.end));
                        Some(if (r1 - r0).abs() <= self.tol / 8.0 {
                            Surface::Cylinder(Cylinder::new(*frame, 0.5 * (r0 + r1))?)
                        } else {
                            Surface::Cone(Cone::new(
                                level(z0)?,
                                r0,
                                ((r1 - r0) / (z1 - z0)).atan(),
                            )?)
                        })
                    }
                    &Form::Arc { center, radius, .. } | &Form::Circle { center, radius, .. } => {
                        let (rho, z) = place(center);
                        (rho - radius > self.tol)
                            .then(|| Torus::new(level(z)?, rho, radius).map_err(KernelError::from))
                            .transpose()?
                            .map(Surface::Torus)
                    }
                    Form::Spline { .. } => None,
                };
                match analytic {
                    Some(surface) => {
                        let (middle, along) = element.middle();
                        let at = station.at(middle);
                        let onward = frame.z().cross(at - frame.origin());
                        let outward = station.vector(along).cross(onward);
                        let reversed = against_normal(&surface, at, outward)?;
                        Ok((surface, reversed))
                    }
                    // u goes around the axis and v along the curve: the
                    // normal is to the left of the edge.
                    None => Ok((
                        Surface::Nurbs(revolved(&curve(station)?, frame, *angle)?),
                        !against,
                    )),
                }
            }
            Motion::Follow { stations, band } => {
                let column = stations.iter().map(curve).collect::<Result<Vec<_>, _>>()?;
                let surface = skin(&column, band)?;
                Ok((flat_or(surface, self.tol, element.source, path)?, against))
            }
        }
    }
}

/// How far the sweep can be from the exact one at a kink of the path:
/// where two pieces meet with directions `turn` (the sine) and `same`
/// (the cosine) apart, and the profile reaches `reach` to either side of
/// the path. Infinite when the path turns back (`same` not positive).
///
/// The exact sweep through a corner turns the profile about the path's
/// binormal there, from square to the piece before to square to the piece
/// after. The sweep instead keeps the profile square to the piece before
/// until a line lays it square again (see the module documentation).
/// Turning the profile about the binormal through a small angle `a` moves
/// a point of it along the path, which is along the faces either piece
/// sweeps, to first order; off them it moves by `reach (1 - cos a)` at
/// most. That second-order movement is what is measured: below half the
/// tolerance the kink is invisible to the body, while the first-order
/// `reach sin a` (a kink of 3e-6 radians across a profile 5 wide, as an
/// end rounded to four decimals gives, is 1.5e-5) is not a distance any
/// point of the body is off.
fn kink_movement(turn: f64, same: f64, reach: f64) -> f64 {
    if same.is_nan() || same <= 0.0 {
        return f64::INFINITY;
    }
    // 1 - cos a, without the cancellation.
    let versine = turn * turn / (1.0 + same);
    let moved = reach * versine;
    if moved.is_nan() { f64::INFINITY } else { moved }
}

/// How far the sweep along an arc of `radius` is from the exact one when
/// the profile arrives turned from square to the arc by an angle whose
/// versine (1 - cos) is `versine`, turning to the left when `side` is
/// positive; `reach` is how far the profile reaches to the right and to
/// the left of the path (`low` and `high`).
///
/// The profile is carried round the arc's own axis as it arrives. A point
/// of it `q` towards the axis, square, is `radius - q` from the axis;
/// turned, `sqrt((radius - q cos)^2 + (q sin)^2)`, which differs by
/// `radius q (1 - cos) / (radius - q)`, to within the square of that. The
/// faces of a curve of the profile that is turned like this are surfaces
/// of revolution of the turned curve, so they are that far from the exact
/// sweep's: exactly so for those that are B-spline surfaces of
/// revolution; tori, cylinders and cones are put through the turned
/// curve's centre or ends, and are off it by no more.
fn tilted_turn(radius: f64, versine: f64, side: f64, reach: (f64, f64)) -> f64 {
    if versine == 0.0 {
        return 0.0;
    }
    let (inside, outside) = if side > 0.0 {
        (reach.1, reach.0)
    } else {
        (-reach.0, -reach.1)
    };
    let mut most: f64 = 0.0;
    for q in [inside, outside] {
        let off = radius * q.abs() * versine / (radius - q);
        if !off.is_finite() {
            return f64::INFINITY;
        }
        most = most.max(off);
    }
    most
}

/// How far `map` moves the profile of `plane` when it lays it into the
/// plane of `home`: the largest distance between a point of the profile
/// where it was drawn and where the sweep puts it, over the box `bbox`
/// (`[min x, min y, max x, max y]` in the profile's sketch) that contains
/// it. The map is affine, so that distance is a convex function of the
/// point and the largest is at a corner of the box. NaN when a corner is
/// not finite.
///
/// When the path crosses the profile's plane at a right angle the map is
/// a rigid motion within one plane and this is a rounding error. When the
/// path leans by an angle `a`, the map takes the sketch's origin to its
/// foot on the square plane and the sketch's x axis to the direction of
/// its shadow there, rigidly: a point moves by about its distance from the
/// sketch's origin times `sin a`.
fn laid_in(map: &Map2, home: &Station, plane: &Frame, bbox: [f64; 4]) -> f64 {
    let [x0, y0, x1, y1] = bbox;
    let mut most: f64 = 0.0;
    for p in [[x0, y0], [x1, y0], [x0, y1], [x1, y1]] {
        let moved = plane.point(p[0], p[1], 0.0).distance(home.at(map.at(p)));
        if moved.is_nan() {
            return f64::NAN;
        }
        most = most.max(moved);
    }
    most
}

/// How two pieces of the path meet.
#[derive(Clone, Copy)]
enum Joint {
    /// The path ends here.
    End,
    /// Tangent: the profile is square to both.
    Smooth,
    /// A corner between two lines: the profile ends on the plane through
    /// it with this normal.
    Mitre(Vec3),
}

/// Sweeps `profile`, which lies in `profile_plane`, along the curves
/// `path` of a sketch in `path_plane`.
///
/// See the module documentation for what a path is, where the profile has
/// to be and what the faces are. The profile may have holes; they are
/// swept with it. The body is a solid that has passed
/// `craft3d_topo::check` with `tolerance`: one shell for an open path;
/// for a closed one a shell per loop of the profile (a hole gives a
/// cavity that goes round). It can still pass through itself: see the
/// module documentation.
///
/// Entities are made in a fixed order: the vertices and edges of the
/// profile where the sweep starts; then per piece of the path those where
/// the piece ends, the edges its vertices sweep and its side faces, loop
/// by loop; then the faces of the lines that sweep one plane; then the
/// start cap and the end cap.
pub fn sweep(
    profile: &Profile,
    profile_plane: &Frame,
    path: &[SketchCurve],
    path_plane: &Frame,
    tolerance: &Tolerance,
) -> Result<Swept, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if profile.outer.segments.is_empty() {
        return Err(KernelError::NoProfile);
    }
    each_curve_once(std::slice::from_ref(profile))?;
    let center = profile
        .centroid()
        .map(|c| profile_plane.point(c[0], c[1], 0.0))
        .filter(|c| c.is_finite())
        .ok_or(KernelError::InvalidProfile {
            reason: "the profile encloses no area",
        })?;

    let lift = |p: P2| path_plane.point(p[0], p[1], 0.0);
    let direction = |d: P2| path_plane.vector(d[0], d[1], 0.0);
    let b_axis = path_plane.z();
    let chain = chain(path, tol)?;
    let height = |p: P2| (lift(p) - profile_plane.origin()).dot(profile_plane.z());
    let away = |p: P2| lift(p).distance(center);
    let Start {
        legs,
        closed,
        origin,
        tangent,
    } = start(chain, &height, &away, tol)?;
    let count = legs.len();
    let all: Vec<u64> = legs.iter().map(|l| l.source).collect();

    // The frame the path carries, where it passes through the profile's
    // plane, and the profile in it. A profile whose plane is not quite
    // square to the path is laid into the plane that is (see `laid_in`).
    let along = direction(tangent);
    let lean = profile_plane.z().cross(along).length();
    let facing = profile_plane.z().dot(along);
    let leans = |moved: Option<f64>| {
        let degrees = lean.clamp(0.0, 1.0).asin().to_degrees();
        let moved = moved.map_or(String::new(), |moved| {
            format!(
                ", and laying the profile square to it would move it by up to {moved}, more than the tolerance {tol}"
            )
        });
        refuse(
            &all,
            format!(
                "the path does not cross the plane of the profile at a right angle: where it passes through it, it leans {degrees}° away from square to it{moved}; draw the profile on a plane square to the path there (a datum plane through the end of the path, normal to it)"
            ),
        )
    };
    if !lean.is_finite() {
        return Err(leans(None));
    }
    let home = Station {
        origin: lift(origin),
        u: direction(left(tangent)),
        b: b_axis,
        square: true,
    };
    let Some(x) = unit(home.local(home.origin + profile_plane.x())) else {
        return Err(leans(None));
    };
    let mirrored = facing < 0.0;
    let map = Map2 {
        origin: home.local(profile_plane.origin()),
        x,
        y: if mirrored {
            scaled(left(x), -1.0)
        } else {
            left(x)
        },
        mirrored,
    };
    let moved = laid_in(&map, &home, profile_plane, profile.outer.bbox());
    if moved.is_nan() || moved > tol {
        return Err(leans(Some(moved)));
    }
    let follows = legs.iter().any(|l| matches!(l.track, Track::Spline(_)));
    let mut loops = profile_loops(profile, &map, tol)?;
    // A whole circle has no seam along lines and arcs; a surface along a
    // spline does not close on itself.
    if follows {
        for elements in &mut loops {
            *elements = elements.iter().flat_map(quarters).collect();
        }
    }
    let sides: usize = loops.iter().map(Vec::len).sum();
    if sides.saturating_mul(count) > MAX_SWEEP_FACES {
        return Err(KernelError::TooManyCurves {
            count: sides.saturating_mul(count),
            limit: MAX_SWEEP_FACES,
        });
    }
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for element in loops.first().into_iter().flatten() {
        let (least, most) = element.reach();
        (low, high) = (low.min(least), high.max(most));
    }
    if !(low.is_finite() && high.is_finite()) {
        return Err(KernelError::InvalidProfile {
            reason: "a point of the profile is not finite",
        });
    }
    // How far the profile reaches to either side of the path, in its
    // plane.
    let reach = low.abs().max(high.abs());

    // How the pieces meet.
    let mut joints = Vec::with_capacity(count);
    for (k, leg) in legs.iter().enumerate() {
        let Some(next) = (if k + 1 < count {
            legs.get(k + 1)
        } else if closed {
            legs.first()
        } else {
            None
        }) else {
            joints.push(Joint::End);
            continue;
        };
        let pair = [leg.source, next.source];
        let (Some(arriving), Some(leaving)) = (leg.tangent(true), next.tangent(false)) else {
            return Err(KernelError::DegenerateCurve {
                id: leg.source,
                reason: "the curve has no direction at its end",
            });
        };
        let (turn, same) = (cross(arriving, leaving), dot(arriving, leaving));
        let lines = leg.is_line() && next.is_line();
        // A kink the profile can be turned through within the tolerance
        // is taken as tangent (see `kink_movement`); between two lines
        // only one within `SMOOTH`, as a mitre is exact.
        let movement = kink_movement(turn, same, reach);
        let smooth = movement <= tol / 2.0 && (!lines || turn.abs() <= SMOOTH);
        joints.push(if smooth {
            Joint::Smooth
        } else if lines {
            // The cosine of half the angle the path turns through.
            let half = ((1.0 + same) / 2.0).max(0.0).sqrt();
            let normal = unit(add(arriving, leaving)).filter(|_| half > 1e-3);
            let Some(normal) = normal else {
                return Err(refuse(
                    &pair,
                    format!(
                        "the path turns back on itself where e{} meets e{}; a profile cannot be carried round a corner of 180°",
                        pair[0], pair[1]
                    ),
                ));
            };
            Joint::Mitre(direction(normal))
        } else {
            return Err(refuse(
                &pair,
                format!(
                    "the path has a corner of {}° where e{} meets e{}, and they are not two straight lines: only a corner between two lines is mitred; make the curves tangent there",
                    turn.atan2(same).abs().to_degrees(),
                    pair[0],
                    pair[1]
                ),
            ));
        });
    }
    let joint = |k: usize| joints.get(k).copied().ok_or_else(internal);

    // The profile laid onto the plane through `at` with normal `normal`
    // by sliding it along `along`.
    let onto = |station: &Station, along: Vec3, at: Vec3, normal: Vec3, square: bool| {
        let slope = along.dot(normal);
        let u = station.u - along * (station.u.dot(normal) / slope);
        let u = if square { u.normalized()? } else { u };
        (slope.is_finite() && slope > 0.0 && u.is_finite()).then_some(Station {
            origin: at,
            u,
            b: station.b,
            square,
        })
    };
    let first_leg = legs.first().ok_or_else(internal)?;
    let mut station = {
        let leaving = first_leg.tangent(false).ok_or_else(internal)?;
        let square = Station {
            origin: lift(first_leg.start),
            u: direction(left(leaving)),
            b: b_axis,
            square: true,
        };
        match (closed, joint(count - 1)?) {
            (true, Joint::Mitre(normal)) => {
                let last = legs.last().ok_or_else(internal)?;
                let arriving = last.tangent(true).ok_or_else(internal)?;
                let before = Station {
                    u: direction(left(arriving)),
                    ..square
                };
                onto(&before, direction(arriving), square.origin, normal, false)
                    .ok_or_else(internal)?
            }
            // A curve that closes the path at a kink into a line arrives
            // square to itself, and the line slides from there.
            (true, Joint::Smooth) if first_leg.is_line() => {
                let last = legs.last().ok_or_else(internal)?;
                if last.is_line() {
                    square
                } else {
                    let arriving = last.tangent(true).ok_or_else(internal)?;
                    Station {
                        u: direction(left(arriving)),
                        ..square
                    }
                }
            }
            _ => square,
        }
    };

    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        faces: Vec::new(),
        tol,
    };
    let first_station = station;
    let first_ring = b.ring(&loops, &station, !closed)?;
    let mut ring = first_ring.clone();
    // Per loop, per vertex, the edges it sweeps, piece by piece.
    let mut rails: Vec<Vec<Vec<(EdgeId, f64)>>> = loops
        .iter()
        .map(|elements| vec![Vec::new(); elements.len()])
        .collect();
    let mut groups: Vec<Vec<FaceId>> = vec![Vec::new(); loops.len()];
    let (mut path_length, mut deviation, mut most_stations) = (0.0, 0.0_f64, 0);

    for (k, leg) in legs.iter().enumerate() {
        let last = k + 1 == count;
        let leaving = leg.tangent(false).ok_or_else(internal)?;
        let (motion, next) = match &leg.track {
            Track::Line => {
                let along = direction(leaving);
                let run = distance(leg.start, leg.end);
                let end = station.origin + along * run;
                // At a kink the line ends square to the piece after it,
                // which then starts square.
                let (normal, square) = match joint(k)? {
                    Joint::Mitre(normal) => (normal, false),
                    Joint::End => (along, true),
                    Joint::Smooth => {
                        let after = legs.get(k + 1).or(legs.first().filter(|_| closed && last));
                        let leaving = after.and_then(|l| l.tangent(false)).ok_or_else(internal)?;
                        (direction(leaving), true)
                    }
                };
                let next = onto(&station, along, end, normal, false).ok_or_else(internal)?;
                // Laid square, a profile that arrives turned (through a
                // kink before the line, or to leave square through one
                // after it) is drawn in towards the path by the cosine of
                // the angle: by `reach (1 - cos)`, which the kinks keep
                // within half the tolerance.
                let next = if square {
                    let stretch = next.u.length();
                    let drawn_in = reach * (stretch - 1.0).abs();
                    if drawn_in.is_nan() || drawn_in > tol / 2.0 {
                        return Err(refuse(
                            &[leg.source],
                            format!(
                                "the path has corners about e{} that together turn the profile too far from square: laying it square at the end of e{} moves it by up to {drawn_in}, more than half the tolerance ({}); make the curves tangent there",
                                leg.source,
                                leg.source,
                                tol / 2.0
                            ),
                        ));
                    }
                    Station {
                        u: next.u.normalized().ok_or_else(internal)?,
                        square: true,
                        ..next
                    }
                } else {
                    next
                };
                // How far each side of the profile travels along the line.
                let shift = (next.u - station.u).dot(along);
                let (short, at) = if shift < 0.0 {
                    (run + high * shift, high)
                } else {
                    (run + low * shift, low)
                };
                if short.is_nan() || short <= tol {
                    let before = if k > 0 {
                        legs.get(k - 1)
                    } else {
                        legs.last().filter(|_| closed)
                    };
                    let after = legs.get(k + 1).or(legs.first().filter(|_| closed && last));
                    // The corner that takes the most of the line there.
                    let at_start = station.u.dot(along) * at > -(next.u.dot(along) * at);
                    let other = if at_start { before } else { after }.unwrap_or(leg);
                    let pair = if at_start {
                        [other.source, leg.source]
                    } else {
                        [leg.source, other.source]
                    };
                    return Err(refuse(
                        &pair,
                        format!(
                            "the corner between e{} and e{} is too sharp for the profile: the profile reaches {} to the side of the path there, and its mitres leave {} of e{}, which is {run} long; make the corner blunter, the line longer or the profile narrower, or round the corner with an arc",
                            pair[0],
                            pair[1],
                            at.abs(),
                            short.min(run),
                            leg.source
                        ),
                    ));
                }
                path_length += run;
                (Motion::Slide { along }, next)
            }
            &Track::Arc { center, sweep } => {
                if !station.square {
                    return Err(internal());
                }
                let side = sweep.signum();
                let radius = distance(leg.start, center);
                // The profile reaches `high` to the left and `-low` to
                // the right; the centre is to the left of a left turn.
                let inside = if side > 0.0 { high } else { -low };
                if inside.is_nan() || radius - inside <= tol {
                    return Err(refuse(
                        &[leg.source],
                        format!(
                            "the bend e{} is too tight for the profile: its radius is {radius}, and the profile reaches {inside} towards its centre; the sweep would fold over itself",
                            leg.source
                        ),
                    ));
                }
                // The profile arrives square to the piece before; after
                // a kink it is turned from square to this arc by `tilt`,
                // and is carried round the arc's own axis like that (see
                // `tilted_turn`).
                let square_here = direction(left(leaving));
                let (sin, cos) = (
                    station.u.cross(square_here).length(),
                    station.u.dot(square_here),
                );
                let versine = sin * sin / (1.0 + cos);
                let off = tilted_turn(radius, versine, side, (low, high));
                if !(cos > 0.0 && off <= tol / 2.0) {
                    let before = if k > 0 {
                        legs.get(k - 1)
                    } else {
                        legs.last().filter(|_| closed)
                    };
                    let pair = [before.map_or(leg.source, |l| l.source), leg.source];
                    return Err(refuse(
                        &pair,
                        format!(
                            "the path has a corner of {}° where e{} meets e{}: carried round e{} with the profile square to the curve before it, the sweep would be up to {off} off, more than half the tolerance ({}); make the curves tangent there",
                            sin.atan2(cos).to_degrees(),
                            pair[0],
                            pair[1],
                            leg.source,
                            tol / 2.0
                        ),
                    ));
                }
                // The end of the path is square to it, within the tolerance.
                if !closed && last {
                    let skew = reach * sin;
                    if skew.is_nan() || skew > tol / 2.0 {
                        return Err(refuse(
                            &[leg.source],
                            format!(
                                "the path ends on e{} turned {}° from square to it, by the corners before it: its end would be up to {skew} off square, more than half the tolerance ({}); make the curves tangent there",
                                leg.source,
                                sin.atan2(cos).to_degrees(),
                                tol / 2.0
                            ),
                        ));
                    }
                }
                let axis = station.origin + square_here * (side * radius);
                let frame =
                    Frame::new(axis, b_axis * side, -square_here * side).ok_or_else(internal)?;
                let angle = sweep.abs();
                let next = Station {
                    origin: axis + turned(&frame, station.origin - axis, angle),
                    u: turned(&frame, station.u, angle)
                        .normalized()
                        .ok_or_else(internal)?,
                    b: b_axis,
                    square: true,
                };
                path_length += radius * angle;
                (Motion::Turn { frame, angle }, next)
            }
            Track::Spline(spline) => {
                if !station.square {
                    return Err(internal());
                }
                let followed = follow(
                    spline,
                    leg.source,
                    &station,
                    path_plane,
                    &loops,
                    (low, high),
                    tol,
                )?;
                let next = *followed.stations.last().ok_or_else(internal)?;
                path_length += followed.length;
                deviation = deviation.max(followed.deviation);
                most_stations = most_stations.max(followed.count);
                (
                    Motion::Follow {
                        stations: followed.stations,
                        band: followed.band,
                    },
                    next,
                )
            }
        };
        let next_ring = if closed && last {
            // Round the path the profile is back where it started.
            let gap = loops
                .iter()
                .flatten()
                .map(|e| next.at(e.start).distance(first_station.at(e.start)))
                .fold(0.0, f64::max);
            if gap.is_nan() || gap > tol / 4.0 {
                return Err(refuse(
                    &all,
                    format!(
                        "the path does not close on itself: followed round, the profile comes back {gap} from where it started; make the curves of the path meet end to end and tangent to each other"
                    ),
                ));
            }
            first_ring.clone()
        } else {
            b.ring(&loops, &next, last)?
        };

        for (l, elements) in loops.iter().enumerate() {
            let (Some(from), Some(to), Some(low_edges), Some(high_edges)) = (
                ring.vertices.get(l),
                next_ring.vertices.get(l),
                ring.edges.get(l),
                next_ring.edges.get(l),
            ) else {
                return Err(internal());
            };
            let seamless = matches!(elements.as_slice(), [only] if only.whole());
            let mut swept: Vec<EdgeId> = Vec::with_capacity(elements.len());
            if !seamless {
                let store = rails.get_mut(l).ok_or_else(internal)?;
                for ((element, (&here, &there)), slot) in
                    elements.iter().zip(from.iter().zip(to)).zip(store)
                {
                    let made = b.rail(&motion, element.start, here, there)?;
                    swept.push(made.0);
                    slot.push(made);
                }
            }
            for (j, element) in elements.iter().enumerate() {
                if element.flat {
                    continue;
                }
                let (Some(&Some((bottom, bottom_against))), Some(&Some((top, top_against)))) =
                    (low_edges.get(j), high_edges.get(j))
                else {
                    return Err(internal());
                };
                // Seen from outside, which is to the right of the loop:
                // along the bottom, up, back along the top, down.
                let around = if seamless {
                    vec![
                        vec![coedge(bottom, bottom_against)],
                        vec![coedge(top, !top_against)],
                    ]
                } else {
                    let (Some(&here), Some(&there)) =
                        (swept.get(j), swept.get((j + 1) % elements.len()))
                    else {
                        return Err(internal());
                    };
                    vec![vec![
                        coedge(bottom, bottom_against),
                        coedge(there, false),
                        coedge(top, !top_against),
                        coedge(here, true),
                    ]]
                };
                let (surface, reversed) =
                    b.surface(&motion, element, &station, &next, leg.source)?;
                let face = b.face(
                    surface,
                    reversed,
                    around,
                    Role::Side,
                    vec![element.source, leg.source],
                );
                groups.get_mut(l).ok_or_else(internal)?.push(face);
            }
        }
        station = next;
        ring = next_ring;
    }

    // The lines that sweep one plane along the whole path.
    for (l, elements) in loops.iter().enumerate() {
        for (j, element) in elements.iter().enumerate() {
            if !element.flat {
                continue;
            }
            let after = (j + 1) % elements.len();
            let (Some(near), Some(far)) = (
                rails.get(l).and_then(|r| r.get(j)),
                rails.get(l).and_then(|r| r.get(after)),
            ) else {
                return Err(internal());
            };
            let onward = far.iter().map(|(edge, _)| coedge(*edge, false));
            let back = near.iter().rev().map(|(edge, _)| coedge(*edge, true));
            let around = if closed {
                // The longer way round is the outside of the face.
                let run = |edges: &[(EdgeId, f64)]| edges.iter().map(|(_, l)| *l).sum::<f64>();
                if run(far) >= run(near) {
                    vec![onward.collect(), back.collect()]
                } else {
                    vec![back.collect(), onward.collect()]
                }
            } else {
                let edge = |ring: &Ring| {
                    ring.edges
                        .get(l)
                        .and_then(|e| e.get(j))
                        .copied()
                        .flatten()
                        .ok_or_else(internal)
                };
                let (bottom, top) = (edge(&first_ring)?, edge(&ring)?);
                let mut around = vec![coedge(bottom.0, bottom.1)];
                around.extend(onward);
                around.push(coedge(top.0, !top.1));
                around.extend(back);
                vec![around]
            };
            let p = first_station.at(element.start);
            let along = first_station.at(element.end) - p;
            let frame =
                Frame::new(p, along.cross(first_station.forward()?), along).ok_or_else(internal)?;
            let face = b.face(
                Surface::Plane(Plane::new(frame)),
                false,
                around,
                Role::Side,
                vec![element.source],
            );
            groups.get_mut(l).ok_or_else(internal)?.push(face);
        }
    }

    if closed {
        for group in groups {
            b.body.add_shell(group);
        }
    } else {
        let mut shell: Vec<FaceId> = groups.into_iter().flatten().collect();
        let sources = cap_sources(&profile.outer);
        for (role, at, ring) in [
            (Role::StartCap, &first_station, &first_ring),
            (Role::EndCap, &station, &ring),
        ] {
            let forward = at.forward()?;
            // The start cap looks back along the path, the end cap along
            // it; seen from outside the start cap its loops run the other
            // way round.
            let start = role == Role::StartCap;
            let normal = if start { -forward } else { forward };
            let frame = Frame::new(at.origin, normal, at.u).ok_or_else(internal)?;
            let mut around = Vec::with_capacity(ring.edges.len());
            for edges in &ring.edges {
                let edges = edges
                    .iter()
                    .map(|e| e.ok_or_else(internal))
                    .collect::<Result<Vec<_>, _>>()?;
                around.push(if start {
                    edges
                        .iter()
                        .rev()
                        .map(|&(edge, against)| coedge(edge, !against))
                        .collect()
                } else {
                    edges
                        .iter()
                        .map(|&(edge, against)| coedge(edge, against))
                        .collect()
                });
            }
            shell.push(b.face(
                Surface::Plane(Plane::new(frame)),
                false,
                around,
                role,
                sources.clone(),
            ));
        }
        b.body.add_shell(shell);
    }

    // Faces with the same sources are numbered in the order they were
    // made.
    let mut seen: BTreeMap<(Role, Vec<u64>), u32> = BTreeMap::new();
    for origin in &mut b.faces {
        let number = seen
            .entry((origin.role, origin.sources.clone()))
            .or_insert(0);
        origin.piece = *number;
        *number = number.saturating_add(1);
    }

    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sweep",
            defects,
        });
    }
    Ok(Swept {
        body: b.body,
        faces: b.faces,
        closed,
        length: path_length,
        deviation,
        stations: most_stations,
    })
}
