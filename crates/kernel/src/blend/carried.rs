//! A chamfer in place whose cut line runs past a smooth edge of its face
//! onto the face beyond.
//!
//! **What it is.** A straight edge between planes and cylinders along it
//! is chamfered in place ([`super::in_place`]): in every plane across the
//! edge the cut line on each face is the point of it at the straight
//! distance from the edge that the distance asked for gives on that face
//! (its chord on a cylinder), and the chamfer face is the plane through
//! the two cut lines. Where a face ends at an edge along the edge before
//! that point, and goes on smoothly into the face beyond (a boss whose
//! wall leaves the top as an arc and goes on as its tangent plane; a top
//! that goes on into an earlier round), the construction carries the cut
//! line across onto the face beyond (`rolling::cross`): the face before
//! goes whole, and the foot is the point of the face beyond at that
//! straight distance. The plan's section holds each face as its whole
//! line or circle, which there is no longer the face, and stands down
//! (`section::enters` with `construct`'s `feet_on_faces`).
//!
//! **What is held here.** In the plane across the edge at its middle each
//! face's trace from the edge to its foot is worked out as the
//! construction takes it: its own face's line or circle up to the edge
//! along which that face ends, then the line or circle of the face beyond
//! up to the point at the straight distance. Both traces and the segment
//! between the feet are lines and arcs, so the rules of a chamfer through
//! a face beside its edge are decided exactly: a point of a trace between
//! the edge and its foot more than the tolerance past the segment's line,
//! between its ends, or, where the trace comes to its foot from past that
//! line, a point of the face beyond the foot more than the tolerance into
//! the region between the feet, shows the region the chamfer changes
//! crossing itself, and no chamfer of the size exists ([`through`]). A
//! configuration not worked out here (a crease where the face ends, the
//! face beyond not a plane or a cylinder along the edge, a foot not on a
//! face) is left to the construction's own checks, as before; where the
//! far edge of the face beyond is not found, that face beyond the foot is
//! not followed, and the rest is held.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::section::{self, Section, Trace};
use super::{Path, Plan, SMOOTH_SINE, Use, face_edges, faces_of, outward, rolling};
use crate::boolean::probe::{OnFace, Probe};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Portable;
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId};
use std::cell::OnceCell;
use std::f64::consts::{PI, TAU};

/// The probe of a body, made the first time it is asked for.
pub(super) type LazyProbe<'a> = OnceCell<Option<Probe<'a>>>;

/// A point of the plane across the edge, in [`Across`]'s coordinates.
type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    let ([ax, ay], [bx, by]) = (a, b);
    [ax - bx, ay - by]
}

fn dot(a: P, b: P) -> f64 {
    let ([ax, ay], [bx, by]) = (a, b);
    ax * bx + ay * by
}

fn length(a: P) -> f64 {
    let [x, y] = a;
    x.phypot(y)
}

fn polar(angle: f64) -> P {
    [angle.pcos(), angle.psin()]
}

fn angle_of(a: P) -> f64 {
    let [x, y] = a;
    y.patan2(x)
}

/// The plane across the edge at its middle: offsets from the edge's point
/// there in two unit directions square to the edge.
struct Across {
    middle: Vec3,
    along: Vec3,
    e1: Vec3,
    e2: Vec3,
}

impl Across {
    /// The point `p` of space, moved along the edge into the plane.
    fn flat(&self, p: Vec3) -> P {
        let offset = p - self.middle;
        [offset.dot(self.e1), offset.dot(self.e2)]
    }

    /// An offset from the edge in the plane, flattened.
    fn offset(&self, v: Vec3) -> P {
        [v.dot(self.e1), v.dot(self.e2)]
    }

    fn lift(&self, p: P) -> Vec3 {
        let [x, y] = p;
        self.e1 * x + self.e2 * y
    }
}

/// A piece of a face's trace.
#[derive(Clone, Copy, Debug)]
enum Piece {
    /// From `a` to `b`.
    Segment { a: P, b: P },
    /// About `centre`, from the angle `start` through the signed `sweep`.
    Arc {
        centre: P,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

/// The trace of the face beyond a smooth edge, from the edge's point `at`
/// on: a line along the unit `dir`, or a circle turned through by the
/// angle the sign of `turn` says. Its parameter is the length along the
/// line, the angle round the circle.
#[derive(Clone, Copy, Debug)]
enum Beyond {
    Line {
        at: P,
        dir: P,
    },
    Circle {
        at: P,
        centre: P,
        radius: f64,
        turn: f64,
    },
}

impl Beyond {
    fn point(&self, t: f64) -> P {
        match *self {
            Beyond::Line { at, dir } => {
                let ([x, y], [dx, dy]) = (at, dir);
                [x + dx * t, y + dy * t]
            }
            Beyond::Circle {
                at,
                centre,
                radius,
                turn,
            } => {
                let [cx, cy] = centre;
                let [ux, uy] = polar(angle_of(sub(at, centre)) + turn * t);
                [cx + radius * ux, cy + radius * uy]
            }
        }
    }

    /// The unit direction of the trace at `t`, onwards.
    fn direction(&self, t: f64) -> P {
        match *self {
            Beyond::Line { dir, .. } => dir,
            Beyond::Circle {
                at, centre, turn, ..
            } => {
                let [ux, uy] = polar(angle_of(sub(at, centre)) + turn * t);
                [-uy * turn, ux * turn]
            }
        }
    }

    fn piece(&self, from: f64, to: f64) -> Piece {
        match *self {
            Beyond::Line { .. } => Piece::Segment {
                a: self.point(from),
                b: self.point(to),
            },
            Beyond::Circle {
                at,
                centre,
                radius,
                turn,
            } => Piece::Arc {
                centre,
                radius,
                start: angle_of(sub(at, centre)) + turn * from,
                sweep: turn * (to - from),
            },
        }
    }

    /// The parameter of `q`, a point within `near` of the trace ahead of
    /// its start (round a circle less than a whole turn).
    fn parameter(&self, q: P, near: f64) -> Option<f64> {
        let t = match *self {
            Beyond::Line { at, dir } => dot(sub(q, at), dir),
            Beyond::Circle {
                at, centre, turn, ..
            } => (turn * (angle_of(sub(q, centre)) - angle_of(sub(at, centre)))).rem_euclid(TAU),
        };
        (t.is_finite() && length(sub(self.point(t), q)) <= near).then_some(t)
    }

    /// The first parameter ahead of the start at which the trace is `d`
    /// (straight) from the edge's point (the origin), within half a turn
    /// round a circle.
    fn at_distance(&self, d: f64) -> Option<f64> {
        match *self {
            Beyond::Line { at, dir } => {
                // |at + t dir|² = d²; `at` lies nearer the edge than `d`.
                let half = dot(at, dir);
                let rest = dot(at, at) - d * d;
                let disc = half * half - rest;
                let t = -half + disc.sqrt();
                (rest < 0.0 && t.is_finite() && t > 0.0).then_some(t)
            }
            Beyond::Circle {
                at,
                centre,
                radius,
                turn,
            } => {
                // |c + r u(φ)|² = d²: c · u(φ) = (d² - |c|² - r²) / 2r.
                let reach = length(centre);
                let k = (d * d - dot(centre, centre) - radius * radius) / (2.0 * radius);
                let ratio = k / reach;
                if !(ratio.abs() <= 1.0 && reach > 0.0) {
                    return None;
                }
                let base = angle_of(centre);
                let spread = ratio.pacos();
                let from = angle_of(sub(at, centre));
                [base + spread, base - spread]
                    .into_iter()
                    .map(|phi| (turn * (phi - from)).rem_euclid(TAU))
                    .filter(|t| *t > 0.0 && *t <= PI)
                    .min_by(f64::total_cmp)
            }
        }
    }
}

/// The trace of a face along the edge that the plan works out, as a
/// curve in the plane across the edge through the edge's point.
fn plain(plan: &Plan, side: usize) -> Option<Trace> {
    match &plan.section {
        Some(section) => section.traces.get(side).copied(),
        None => plan.into.get(side).map(|into| Trace::Line { into: *into }),
    }
}

/// The piece of `trace` (through the edge's point, the origin) from the
/// origin to `to`, `s` along it.
fn opening(across: &Across, trace: &Trace, to: P, s: f64) -> Piece {
    match trace.circle() {
        None => Piece::Segment {
            a: [0.0, 0.0],
            b: to,
        },
        Some((centre, radius)) => {
            let centre = across.offset(centre);
            let start = angle_of(sub([0.0, 0.0], centre));
            let leaves = across.offset(trace.direction(0.0));
            let [ux, uy] = polar(start);
            let turn = if dot([-uy, ux], leaves) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            Piece::Arc {
                centre,
                radius,
                start,
                sweep: turn * s / radius,
            }
        }
    }
}

/// The edges of `face` that are lines along the edge and pass through the
/// plane across it at its middle (a translation along the edge carries
/// the face into itself, so each of them is a place where the face ends
/// in every plane across the edge near the middle), with that point.
fn along_edges(body: &Body, face: FaceId, across: &Across, tol: f64) -> Vec<(EdgeId, P)> {
    let mut found = Vec::new();
    for edge in face_edges(body, face) {
        let Some(data) = body.edge(edge) else {
            continue;
        };
        let Curve3::Line(line) = &data.curve else {
            continue;
        };
        if line.direction().cross(across.along).length() > 1e-9 {
            continue;
        }
        let ends = [data.start, data.end].map(|v| body.vertex(v).map(|v| v.point));
        let [Some(a), Some(b)] = ends else {
            continue;
        };
        let (ta, tb) = (
            (a - across.middle).dot(across.along),
            (b - across.middle).dot(across.along),
        );
        if ta.min(tb) > tol || ta.max(tb) < -tol {
            continue;
        }
        found.push((edge, across.flat(line.origin())));
    }
    found
}

/// A side whose cut line is carried onto the face beyond: its pieces from
/// the edge to the foot, each with its face, and what lies beyond the
/// foot.
struct Carried {
    pieces: [(Piece, FaceId); 2],
    foot: P,
    beyond: Beyond,
    /// The face beyond and the parameter of the foot and of where that
    /// face ends past it (`None` where its far edge is not found).
    after: FaceId,
    at_foot: f64,
    ends: Option<f64>,
}

/// The trace of side `side` carried onto the face beyond, when its face
/// ends along it (at an edge along the edge) short of the plan's foot,
/// with the foot at the straight distance `d` from the edge. `None` where
/// the face does not end short of the foot, or what lies beyond is not
/// worked out here.
fn carried<'a>(
    body: &'a Body,
    uses: &[Vec<Use>],
    probe: &LazyProbe<'a>,
    plan: &Plan,
    side: usize,
    across: &Across,
    tolerance: &Tolerance,
) -> Option<Option<Carried>> {
    let tol = tolerance.linear;
    let face = *plan.faces.get(side)?;
    let trace = plain(plan, side)?;
    let foot = *plan.feet().get(side)?;
    let reach = trace.coordinate(foot);
    let d = foot.length();
    let near = 2.0 * tol;
    // Where the face ends along its trace: the nearest such edge ahead of
    // the edge itself, short of the foot.
    let mut end: Option<(EdgeId, P, f64)> = None;
    for (edge, q) in along_edges(body, face, across, tol) {
        if edge == plan.edge {
            continue;
        }
        let s = trace.coordinate(across.lift(q));
        let on = length(sub(across.offset(trace.at(s)), q)) <= near;
        if on && s > 16.0 * tol && end.is_none_or(|(_, _, best)| s < best) {
            end = Some((edge, q, s));
        }
    }
    let Some((smooth, at, s)) = end else {
        return Some(None);
    };
    if !(s < reach - tol) {
        return Some(None);
    }
    let [a, b] = faces_of(uses, smooth)?;
    let after = if a == face { b } else { a };
    if after == face {
        return None;
    }
    // The trace before the smooth edge goes on into the face beyond.
    let leaving = across.offset(trace.direction(s));
    let beyond = match &body.face(after)?.surface {
        Surface::Plane(plane) => {
            let normal = plane.frame().z();
            if normal.dot(across.along).abs() > 1e-9 {
                return None;
            }
            let dir = across.offset(normal.cross(across.along));
            let size = length(dir);
            if !(size > 0.5) {
                return None;
            }
            let sign = if dot(dir, leaving) >= 0.0 { 1.0 } else { -1.0 };
            let [x, y] = dir;
            Beyond::Line {
                at,
                dir: [sign * x / size, sign * y / size],
            }
        }
        Surface::Cylinder(cylinder) => {
            if cylinder.frame().z().cross(across.along).length() > 1e-9 {
                return None;
            }
            let centre = across.flat(cylinder.frame().origin());
            let radius = cylinder.radius();
            let out = sub(at, centre);
            if !((length(out) - radius).abs() <= near.max(1e-12 * radius)) {
                return None;
            }
            let [ox, oy] = out;
            let ahead = dot([-oy, ox], leaving) / radius;
            if !(ahead.abs() >= 0.5) {
                return None;
            }
            Beyond::Circle {
                at,
                centre,
                radius,
                turn: ahead.signum(),
            }
        }
        Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Bezier(_)
        | Surface::Nurbs(_) => return None,
    };
    // Smooth there, as the construction asks of an edge it carries a
    // border across (`rolling::cross`, ten times `bend`); across a crease
    // it carries nothing, and refuses on its own.
    let [lx, ly] = leaving;
    let [bx, by] = beyond.direction(0.0);
    let bound = 10.0 * rolling::bend(tolerance, plan.largest());
    if !((lx * by - ly * bx).abs() <= bound && lx * bx + ly * by > 0.0) {
        return None;
    }
    let at_foot = beyond.at_distance(d)?;
    let foot = beyond.point(at_foot);
    // The face before is the face there, and the foot lies on the face
    // beyond: as the construction takes them.
    let probe = probe
        .get_or_init(|| Probe::new(body, tolerance).ok())
        .as_ref()?;
    let inside = |face: FaceId, q: P| {
        !matches!(
            probe.on_face(face.0 as usize, across.middle + across.lift(q)),
            Ok(OnFace::Out) | Err(_)
        )
    };
    if !(inside(face, across.offset(trace.at(0.5 * s))) && inside(after, foot)) {
        return None;
    }
    // Where the face beyond ends past the foot.
    let mut ends: Option<f64> = None;
    for (edge, q) in along_edges(body, after, across, tol) {
        if edge == smooth {
            continue;
        }
        if let Some(t) = beyond.parameter(q, near)
            && t > at_foot
            && ends.is_none_or(|best| t < best)
        {
            ends = Some(t);
        }
    }
    Some(Some(Carried {
        pieces: [
            (opening(across, &trace, at, s), face),
            (beyond.piece(0.0, at_foot), after),
        ],
        foot,
        beyond,
        after,
        at_foot,
        ends,
    }))
}

/// How deep into the region between the feet the piece runs from its
/// start (a foot, on the segment's line through `f0` along the unit `w`,
/// `len` long, whose unit normal `n` points away from the edge): followed
/// from there while it lies on the edge's side of the line and between the
/// lines square to the segment through its ends, the largest of
/// `-(x - f0) · n` on that stretch; zero where it leaves at once. Where it
/// leaves, it is out of the region (back past the line, or beyond a foot);
/// what comes after is not followed.
fn runs_in(piece: &Piece, f0: P, n: P, w: P, len: f64) -> f64 {
    // The piece by a parameter from 0 (its start) to `most`.
    let (most, at): (f64, Box<dyn Fn(f64) -> P>) = match *piece {
        Piece::Segment { a, b } => (
            1.0,
            Box::new(move |t: f64| {
                let ([ax, ay], [bx, by]) = (a, b);
                [ax + (bx - ax) * t, ay + (by - ay) * t]
            }),
        ),
        Piece::Arc {
            centre,
            radius,
            start,
            sweep,
        } => (
            sweep.abs(),
            Box::new(move |t: f64| {
                let ([cx, cy], [ux, uy]) = (centre, polar(start + sweep.signum() * t));
                [cx + radius * ux, cy + radius * uy]
            }),
        ),
    };
    let g = |t: f64| dot(sub(at(t), f0), w);
    let h = |t: f64| dot(sub(at(t), f0), n);
    // Where the piece meets one of the three lines, past its start.
    let mut meets: Vec<f64> = Vec::new();
    match *piece {
        Piece::Segment { a, b } => {
            for (direction, levels) in [(w, [0.0, len]), (n, [0.0, 0.0])] {
                let (value, toward) = (dot(sub(a, f0), direction), dot(sub(b, a), direction));
                for level in levels {
                    if toward != 0.0 {
                        meets.push((level - value) / toward);
                    }
                }
            }
        }
        Piece::Arc {
            centre,
            radius,
            start,
            sweep,
        } => {
            for (direction, levels) in [(w, [0.0, len]), (n, [0.0, 0.0])] {
                let base = dot(sub(centre, f0), direction);
                for level in levels {
                    let ratio = (level - base) / radius;
                    if ratio.abs() <= 1.0 {
                        let spread = ratio.pacos();
                        for phi in [angle_of(direction) + spread, angle_of(direction) - spread] {
                            meets.push((sweep.signum() * (phi - start)).rem_euclid(TAU));
                        }
                    }
                }
            }
        }
    }
    let first = 1e-9 * most;
    let leaves = meets
        .into_iter()
        .filter(|t| *t > first && *t < most)
        .fold(most, f64::min);
    let middle = 0.5 * leaves;
    let gm = g(middle);
    if !(gm >= 0.0 && gm <= len && h(middle) <= 0.0) {
        return 0.0;
    }
    let mut deepest = -h(leaves);
    if let Piece::Arc { start, sweep, .. } = *piece {
        let [nx, ny] = n;
        let t = (sweep.signum() * (angle_of([-nx, -ny]) - start)).rem_euclid(TAU);
        if t > 0.0 && t < leaves {
            deepest = deepest.max(-h(t));
        }
    }
    deepest.max(0.0)
}

/// How far the piece reaches past the line through `f0` along the unit
/// `w` (towards its unit normal `n`, away from the edge), at its points
/// between the lines square to the segment through `f0` and `f0 + len w`:
/// the largest of `(x - f0) · n` there, minus infinity where none of it
/// lies there.
fn reach(piece: &Piece, f0: P, n: P, w: P, len: f64) -> f64 {
    let strip = |x: P| {
        let g = dot(sub(x, f0), w);
        g >= -1e-12 * len && g <= len * (1.0 + 1e-12)
    };
    let height = |x: P| dot(sub(x, f0), n);
    match *piece {
        Piece::Segment { a, b } => {
            let (ga, gb) = (dot(sub(a, f0), w), dot(sub(b, f0), w));
            let at = |t: f64| {
                let ([ax, ay], [bx, by]) = (a, b);
                [ax + (bx - ax) * t, ay + (by - ay) * t]
            };
            let mut ts = vec![0.0, 1.0];
            if gb != ga {
                for g in [0.0, len] {
                    let t = (g - ga) / (gb - ga);
                    if t > 0.0 && t < 1.0 {
                        ts.push(t);
                    }
                }
            }
            ts.into_iter()
                .map(at)
                .filter(|x| strip(*x))
                .map(height)
                .fold(f64::NEG_INFINITY, f64::max)
        }
        Piece::Arc {
            centre,
            radius,
            start,
            sweep,
        } => {
            // The largest of a linear function on the arc where it lies
            // in the strip: at an end of the arc, where it crosses a side
            // of the strip, or where the function is largest round the
            // circle.
            let point = |phi: f64| {
                let ([cx, cy], [ux, uy]) = (centre, polar(phi));
                [cx + radius * ux, cy + radius * uy]
            };
            let mut angles = vec![start, start + sweep];
            angles.push(angle_of(n));
            let base = dot(sub(centre, f0), w);
            for g in [0.0, len] {
                let ratio = (g - base) / radius;
                if ratio.abs() <= 1.0 {
                    let spread = ratio.pacos();
                    angles.push(angle_of(w) + spread);
                    angles.push(angle_of(w) - spread);
                }
            }
            let within = |phi: f64| {
                let ahead = (sweep.signum() * (phi - start)).rem_euclid(TAU);
                ahead <= sweep.abs() || (TAU - ahead) <= 1e-12
            };
            angles
                .into_iter()
                .filter(|phi| within(*phi))
                .map(point)
                .filter(|x| strip(*x))
                .map(height)
                .fold(f64::NEG_INFINITY, f64::max)
        }
    }
}

/// The face the chamfer of `plan` (a chamfer built in place: in its plane
/// across the edge, the cut lines at the straight distances `distances`)
/// is shown to pass through, where its cut line on a side is carried
/// across a smooth edge of that side's face onto the face beyond (see the
/// module documentation). `None` where nothing is carried, where it is
/// clear of the faces, or where what is carried is not worked out here.
pub(super) fn through<'a>(
    body: &'a Body,
    uses: &[Vec<Use>],
    probe: &LazyProbe<'a>,
    plan: &Plan,
    tolerance: &Tolerance,
) -> Option<FaceId> {
    let tol = tolerance.linear;
    let Path::Line {
        start,
        along,
        length: edge_length,
    } = plan.path
    else {
        return None;
    };
    if plan.round.is_some() {
        return None;
    }
    let [into, _] = plan.into;
    let e1 = (into - along * into.dot(along)).normalized()?;
    let across = Across {
        middle: start + along * (0.5 * edge_length),
        along,
        e1,
        e2: along.cross(e1),
    };
    let mut sides = [None, None];
    for (side, slot) in sides.iter_mut().enumerate() {
        *slot = carried(body, uses, probe, plan, side, &across, tolerance)?;
    }
    if sides.iter().all(Option::is_none) {
        return None;
    }
    let plan_feet = plan.feet();
    let feet: Vec<P> = sides
        .iter()
        .zip(plan_feet)
        .map(|(c, f)| c.as_ref().map_or_else(|| across.offset(f), |c| c.foot))
        .collect();
    let [f0, f1] = <[P; 2]>::try_from(feet).ok()?;
    let len = length(sub(f1, f0));
    if !(len > 16.0 * tol) {
        return None;
    }
    let [wx, wy] = sub(f1, f0);
    let w = [wx / len, wy / len];
    let mut n = [-w[1], w[0]];
    if dot(n, f0) < 0.0 {
        n = [w[1], -w[0]];
    }
    // The edge's point (the origin) lies on the other side of the line.
    if !(dot(n, f0) > tol) {
        return None;
    }
    let feet3 = [across.lift(f0), across.lift(f1)];
    for (side, carried) in sides.iter().enumerate() {
        match carried {
            Some(c) => {
                // The trace from the edge to the foot past the segment.
                for (piece, face) in &c.pieces {
                    if reach(piece, f0, n, w, len) > tol {
                        return Some(*face);
                    }
                }
                // Coming to the foot from past the line (or along it): the
                // face beyond the foot in the region between the feet; not
                // looked at where that face's far edge is not found (the
                // other side is still held).
                let arrives = c.beyond.direction(c.at_foot);
                if dot(arrives, n) < SMOOTH_SINE
                    && let Some(ends) = c.ends
                    && runs_in(&c.beyond.piece(c.at_foot, ends), f0, n, w, len) > tol
                {
                    return Some(c.after);
                }
            }
            None => {
                // The face's own line or circle through the edge, held as
                // the plan holds it, against the segment to the carried foot.
                let face = *plan.faces.get(side)?;
                let trace = plain(plan, side)?;
                let other = plain(plan, 1 - side)?;
                let traces = if side == 0 {
                    [trace, other]
                } else {
                    [other, trace]
                };
                let normal = outward(body, face, start).ok()?;
                let held = Section {
                    traces,
                    feet: feet3,
                    distances: plan.distances,
                    centre: None,
                };
                if section::enters_side(&held, [normal, normal], plan.convex, tol, side) {
                    return Some(face);
                }
            }
        }
    }
    None
}
