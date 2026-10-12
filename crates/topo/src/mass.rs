//! Area, volume and centroid from the exact B-rep, independent of any
//! tessellation: the backbone of the test oracles (docs/testing.md).
//!
//! Method. By the divergence theorem the volume and its first moments are
//! surface integrals over the faces; with the surface's own parametrisation
//! each becomes `∬_D f(u, v) du dv` over the face's parameter domain `D`:
//!
//! - area:    `f = |Sᵤ × Sᵥ|`
//! - volume:  `f = S · (Sᵤ × Sᵥ) / 3`
//! - moments: `f = S_k² (Sᵤ × Sᵥ)_k / 2` for each coordinate `k`
//!
//! `D` is only known through its boundary, the face's loops. Green's theorem
//! turns the domain integral into a boundary one:
//! `∬_D f du dv = -∮ G du`, with `G(u, v) = ∫ f(u, t) dt` from a reference
//! `v₀` to `v`. Both integrals are Gauss–Legendre sums; the boundary is
//! followed along the edge curves, projected onto the surface.
//!
//! Three things make `D` more than a region of the plane.
//!
//! **Loops around the axis.** On a surface closed around its axis a loop may
//! wind around it (the rim of a cone, the two ends of a cylinder wall). `G`
//! is periodic in u, so nothing changes, except that a face whose boundary
//! winds once in total reaches to the end of the parameter range on one
//! side (a pole or an apex): `v₀` is then that end, where the missing
//! boundary contributes nothing.
//!
//! **Poles and apexes on the boundary.** Where the boundary passes through
//! a pole of a sphere or the apex of a cone (the corner of a sphere octant,
//! the tip of a slice of a cone) that point is, in parameter space, a whole
//! stretch of the edge of the parameter range: u runs from the direction
//! the boundary arrives by to the one it leaves by, and no edge of the body
//! stands for it. That stretch is added to the boundary integral. The face
//! is always on the inner side of it, so it runs towards lower u at the
//! upper end of v and towards higher u at the lower end, through less than
//! a turn.
//!
//! **Surfaces closed both ways (the torus).** There `f du dv` is not the
//! derivative of anything periodic in both directions: `G` gains the
//! integral over a whole turn of the tube each time v goes around. Loops
//! that do not wind around the tube are followed with v unrolled
//! continuously; for loops that do (the ends of a pipe elbow) the roles of
//! u and v are swapped: `∬_D f du dv = ∮ H dv` with
//! `H(u, v) = ∫ f(t, v) dt`, and u unrolled. Either way the result is only
//! determined up to a whole number of times the integral over the entire
//! torus: which turn of the unrolled parameter a loop starts on is
//! arbitrary, and so is which side of its loops the face is on when that
//! is the larger one. The area settles it: it lies between zero and the
//! area of the torus, and the same whole number applies to all five
//! integrals. The same argument gives the part of a sphere *outside* a
//! loop that circles neither pole.

use crate::{Body, BodyKind, Coedge, Face};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::Vec3;
use craft3d_math::quadrature::Rule;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProperties {
    /// Total surface area.
    pub area: f64,
    /// Enclosed volume; 0 for a sheet. Negative means the faces point
    /// inward.
    pub volume: f64,
    /// Centre of volume; `None` for a sheet or a zero volume.
    pub centroid: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MassError {
    /// A face refers to a loop or edge that does not exist; run `check`.
    Invalid {
        face: usize,
    },
    /// A face is not bounded: no loops on an open surface, or a loop that
    /// winds around a surface with no end on that side.
    UnboundedFace {
        face: usize,
    },
    /// A configuration the integrator does not handle yet.
    Unsupported {
        face: usize,
        what: &'static str,
    },
    Geometry {
        face: usize,
        error: GeomError,
    },
}

impl fmt::Display for MassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MassError::Invalid { face } => write!(
                f,
                "face {face} refers to a loop or edge that does not exist; check the body first"
            ),
            MassError::UnboundedFace { face } => {
                write!(f, "face {face} is not bounded by its loops")
            }
            MassError::Unsupported { face, what } => {
                write!(f, "face {face}: not supported yet: {what}")
            }
            MassError::Geometry { face, error } => write!(f, "face {face}: {error}"),
        }
    }
}

impl std::error::Error for MassError {}

/// The five integrands at once: area, volume, and the three moments.
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    area: f64,
    volume: f64,
    moment: Vec3,
}

impl Sums {
    fn add_scaled(&mut self, o: Sums, s: f64) {
        self.area += o.area * s;
        self.volume += o.volume * s;
        self.moment += o.moment * s;
    }
}

/// `f(u, v)` for the five integrals.
fn integrand(surface: &Surface, u: f64, v: f64) -> Sums {
    let (s, su, sv) = surface.derivatives(u, v);
    let n = su.cross(sv);
    Sums {
        area: n.length(),
        volume: s.dot(n) / 3.0,
        moment: Vec3::new(s.x * s.x * n.x, s.y * s.y * n.y, s.z * s.z * n.z) * 0.5,
    }
}

const NODES: usize = 16;
/// Panels for an inner integral, at least.
const INNER_PANELS: usize = 2;
/// And at most: an unrolled parameter stays within a few turns.
const MAX_INNER_PANELS: usize = 64;

/// A vertex this close to a pole or apex, relative to how far the face's
/// boundary reaches from it, is at it. Generous on purpose: two different
/// meridians can only share a vertex at the pole, so a vertex near one is
/// meant to be on it, and taking a true near miss for a touch costs no
/// more than the sliver between the two.
const AT_POLE: f64 = 1e-6;

/// The direction an inner integral runs in.
#[derive(Clone, Copy)]
enum Along {
    U,
    V,
}

/// `∫ f dt` from `from` to `to` along one parameter, the other one fixed.
fn inner(surface: &Surface, rule: &Rule, along: Along, fixed: f64, from: f64, to: f64) -> Sums {
    let periodic = match along {
        Along::U => surface.period_u().is_some(),
        Along::V => surface.period_v().is_some(),
    };
    // Around a closed direction the integrand is trigonometric and the
    // span may be several turns: at most a quarter turn per panel.
    let count = if periodic {
        (((to - from).abs() / FRAC_PI_2).ceil() as usize).clamp(INNER_PANELS, MAX_INNER_PANELS)
    } else {
        INNER_PANELS
    };
    let mut sum = Sums::default();
    let width = (to - from) / count as f64;
    for k in 0..count {
        let start = from + width * k as f64;
        for (t, w) in rule.points(start, start + width) {
            let (u, v) = match along {
                Along::U => (t, fixed),
                Along::V => (fixed, t),
            };
            sum.add_scaled(integrand(surface, u, v), w);
        }
    }
    sum
}

/// How many quadrature panels an edge needs: enough that each spans at most
/// an eighth of a turn or one spline span.
fn panels(curve: &Curve3, range: (f64, f64)) -> usize {
    let length = (range.1 - range.0).abs();
    match curve {
        Curve3::Line(_) => 2,
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            ((length / FRAC_PI_4).ceil() as usize).clamp(1, 64)
        }
        Curve3::Nurbs(_) => 4,
    }
}

/// Where the quadrature panels of a piece of an edge begin and end. A
/// spline is only as smooth as its knots allow: a panel across a knot
/// integrates a function with a kink in it, so the panels end at the knots
/// and divide each span between them.
fn panel_ends(curve: &Curve3, low: f64, high: f64) -> Vec<f64> {
    let mut ends = vec![low];
    if let Curve3::Nurbs(c) = curve {
        const PER_SPAN: usize = 4;
        let mut stops: Vec<f64> = c
            .knots()
            .iter()
            .copied()
            .filter(|&k| k > low && k < high)
            .collect();
        stops.push(high);
        stops.dedup();
        if stops.len() > 4096 {
            stops = vec![high];
        }
        let mut from = low;
        for stop in stops {
            for k in 1..=PER_SPAN {
                ends.push(from + (stop - from) * k as f64 / PER_SPAN as f64);
            }
            from = stop;
        }
        return ends;
    }
    let count = panels(curve, (low, high));
    for k in 1..=count {
        ends.push(low + (high - low) * k as f64 / count as f64);
    }
    ends
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    // rem_euclid can return TAU itself for tiny negative input.
    if w >= TAU { 0.0 } else { w }
}

/// A point where the parametrisation of an analytic surface collapses: an
/// end of the v range that is one point in space.
struct Pole {
    point: Vec3,
    v: f64,
    /// At the upper end of v (else the lower).
    top: bool,
    /// The directions u = 0 and u = π/2 point in.
    x: Vec3,
    y: Vec3,
}

fn poles(surface: &Surface) -> Vec<Pole> {
    match surface {
        Surface::Sphere(s) => {
            let (f, r) = (s.frame(), s.radius());
            [(r, true), (-r, false)]
                .into_iter()
                .map(|(z, top)| Pole {
                    point: f.point(0.0, 0.0, z),
                    v: if top { FRAC_PI_2 } else { -FRAC_PI_2 },
                    top,
                    x: f.x(),
                    y: f.y(),
                })
                .collect()
        }
        Surface::Cone(c) => {
            let (f, v) = (c.frame(), c.apex_v());
            if !v.is_finite() {
                return Vec::new();
            }
            vec![Pole {
                point: f.point(0.0, 0.0, v),
                v,
                top: c.half_angle() < 0.0,
                x: f.x(),
                y: f.y(),
            }]
        }
        Surface::Plane(_)
        | Surface::Cylinder(_)
        | Surface::Torus(_)
        | Surface::Bezier(_)
        | Surface::Nurbs(_) => Vec::new(),
    }
}

/// A stretch of an edge as a loop runs along it, from parameter `start` to
/// `end` (either may be the larger), with the pole it starts or ends at.
struct Piece<'a> {
    curve: &'a Curve3,
    start: f64,
    end: f64,
    start_pole: Option<usize>,
    end_pole: Option<usize>,
}

/// The boundary passing through a pole: the u it arrives by and leaves by.
struct Passage {
    v: f64,
    top: bool,
    u_in: f64,
    u_out: f64,
}

impl Passage {
    /// The direction (±1) u runs in along the pole's stretch of the edge of
    /// the parameter range, and how far: the face is below the upper edge
    /// and above the lower one, and its boundary runs counter-clockwise
    /// when `sense` is +1.
    fn sweep(&self, sense: f64, face: usize) -> Result<(f64, f64), MassError> {
        let direction = if self.top { -sense } else { sense };
        let angle = wrap(direction * (self.u_out - self.u_in));
        if !(angle > 1e-9 && angle < TAU - 1e-9) {
            // No angle or a whole turn: which of the two cannot be told.
            return Err(MassError::Unsupported {
                face,
                what: "a boundary that doubles back on itself at a pole or an apex",
            });
        }
        Ok((direction, angle))
    }
}

/// One boundary sample: surface parameters, and how fast they change along
/// the edge, times the quadrature weight and the traversal direction.
struct Sample {
    /// Which loop of the face it belongs to.
    loop_index: usize,
    u: f64,
    v: f64,
    du: f64,
    dv: f64,
}

struct Boundary {
    /// In the order the loops run.
    samples: Vec<Sample>,
    passages: Vec<Passage>,
    /// Whether the boundary touches a point where a freeform surface's
    /// parametrisation collapses (a collapsed patch edge).
    collapsed: bool,
}

/// The pieces of one loop in the order it runs, split wherever an edge
/// passes through a pole between its ends.
fn loop_pieces<'a>(
    body: &'a Body,
    index: usize,
    coedges: &[Coedge],
    poles: &[Pole],
    reach: &[f64],
) -> Result<Vec<Piece<'a>>, MassError> {
    let invalid = MassError::Invalid { face: index };
    let at = |point: Vec3| {
        poles
            .iter()
            .zip(reach)
            .position(|(pole, &reach)| point.distance(pole.point) <= AT_POLE * reach)
    };
    let mut pieces = Vec::new();
    for &Coedge { edge, reversed } in coedges {
        let edge = body.edge(edge).ok_or(invalid.clone())?;
        let (t0, t1) = edge.range;
        // The vertices decide, not the ends of the curve: two edges that
        // share a vertex then agree on whether it is a pole.
        let (Some(first), Some(last)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
            return Err(invalid);
        };
        let (first, last) = (at(first.point), at(last.point));
        // Where the edge passes through a pole on its way.
        let mut cuts: Vec<(f64, usize)> = Vec::new();
        for (which, (pole, &reach)) in poles.iter().zip(reach).enumerate() {
            if first == Some(which) || last == Some(which) || t1 <= t0 || (t1 - t0).is_nan() {
                continue;
            }
            let mut t = edge
                .curve
                .project(pole.point)
                .map_err(|error| MassError::Geometry { face: index, error })?;
            if let Some(period) = edge.curve.period() {
                t = t0 + (t - t0).rem_euclid(period);
            }
            let margin = 1e-9 * (t1 - t0);
            if t > t0 + margin
                && t < t1 - margin
                && edge.curve.eval(t).distance(pole.point) <= AT_POLE * reach
            {
                cuts.push((t, which));
            }
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut run = Vec::with_capacity(cuts.len() + 1);
        let mut from = (t0, first);
        for (t, which) in cuts {
            run.push(Piece {
                curve: &edge.curve,
                start: from.0,
                end: t,
                start_pole: from.1,
                end_pole: Some(which),
            });
            from = (t, Some(which));
        }
        run.push(Piece {
            curve: &edge.curve,
            start: from.0,
            end: t1,
            start_pole: from.1,
            end_pole: last,
        });
        if reversed {
            run.reverse();
            for piece in &mut run {
                *piece = Piece {
                    curve: piece.curve,
                    start: piece.end,
                    end: piece.start,
                    start_pole: piece.end_pole,
                    end_pole: piece.start_pole,
                };
            }
        }
        pieces.extend(run);
    }
    Ok(pieces)
}

/// The u a curve points along as it passes a pole: the angle of `tangent`
/// about the surface's axis.
fn heading(pole: &Pole, tangent: Vec3, face: usize) -> Result<f64, MassError> {
    let (x, y) = (tangent.dot(pole.x), tangent.dot(pole.y));
    let flat = x.hypot(y);
    if !(flat.is_finite() && flat > 1e-9 * tangent.length()) {
        return Err(MassError::Unsupported {
            face,
            what: "a boundary that has no direction where it passes a pole or an apex",
        });
    }
    Ok(wrap(y.atan2(x)))
}

fn boundary(body: &Body, face: &Face, index: usize, rule: &Rule) -> Result<Boundary, MassError> {
    let invalid = MassError::Invalid { face: index };
    let surface = &face.surface;
    let poles = poles(surface);
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    let mut collapsed = false;

    // How far the boundary reaches from each pole, and whether it touches a
    // collapsed edge of a freeform surface.
    let mut reach = vec![0.0_f64; poles.len()];
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_id in &face.loops {
        let loop_ = body.loop_(*loop_id).ok_or(invalid.clone())?;
        for coedge in &loop_.coedges {
            let edge = body.edge(coedge.edge).ok_or(invalid.clone())?;
            let (t0, t1) = edge.range;
            for t in [t0, 0.5 * (t0 + t1), t1] {
                let point = edge.curve.eval(t);
                for (pole, reach) in poles.iter().zip(&mut reach) {
                    let distance = point.distance(pole.point);
                    if distance.is_finite() {
                        *reach = reach.max(distance);
                    }
                }
            }
            if freeform {
                for end in [t0, t1] {
                    let (u, v) = surface
                        .project(edge.curve.eval(end))
                        .map_err(|error| MassError::Geometry { face: index, error })?;
                    let (_, su, sv) = surface.derivatives(u, v);
                    let scale = su.length() + sv.length();
                    if su.length() <= 1e-9 * scale || sv.length() <= 1e-9 * scale {
                        collapsed = true;
                    }
                }
            }
        }
        loops.push(&loop_.coedges);
    }

    let mut samples = Vec::new();
    let mut passages = Vec::new();
    for (loop_index, coedges) in loops.into_iter().enumerate() {
        let pieces = loop_pieces(body, index, coedges, &poles, &reach)?;
        let after = pieces.iter().cycle().skip(1);
        for (piece, next) in pieces.iter().zip(after) {
            let (low, high) = (piece.start.min(piece.end), piece.start.max(piece.end));
            let direction = if piece.end < piece.start { -1.0 } else { 1.0 };
            let ends = panel_ends(piece.curve, low, high);
            let first = samples.len();
            for (&start, &stop) in ends.iter().zip(ends.iter().skip(1)) {
                for (t, w) in rule.points(start, stop) {
                    let (point, tangent) = piece.curve.derivatives(t);
                    let (u, v) = surface
                        .project(point)
                        .map_err(|error| MassError::Geometry { face: index, error })?;
                    // Solve [Sᵤ Sᵥ] (du, dv) = tangent in the least-squares
                    // sense: the edge lies on the surface, so this is exact
                    // up to the modelling tolerance.
                    let (_, su, sv) = surface.derivatives(u, v);
                    let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
                    let det = a * c - b * b;
                    let (ru, rv) = (tangent.dot(su), tangent.dot(sv));
                    // Next to a pole Sᵤ vanishes and u stops changing: that
                    // is the limit along any curve through it.
                    let (du, dv) = if det > 1e-24 * a.max(c) * a.max(c) && det.is_finite() {
                        ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
                    } else if c > 0.0 {
                        (0.0, rv / c)
                    } else {
                        (0.0, 0.0)
                    };
                    samples.push(Sample {
                        loop_index,
                        u,
                        v,
                        du: du * w * direction,
                        dv: dv * w * direction,
                    });
                }
            }
            if direction < 0.0 {
                // Keep the samples in the order the loop runs.
                if let Some(block) = samples.get_mut(first..) {
                    block.reverse();
                }
            }
            match (piece.end_pole, next.start_pole) {
                (None, None) => {}
                (Some(a), Some(b)) if a == b => {
                    let pole = poles.get(a).ok_or(invalid.clone())?;
                    // Arriving, the curve points away from where it came
                    // from; leaving, towards where it goes.
                    let arriving = piece.curve.derivatives(piece.end).1 * direction;
                    let leaving = next.curve.derivatives(next.start).1
                        * if next.end < next.start { -1.0 } else { 1.0 };
                    passages.push(Passage {
                        v: pole.v,
                        top: pole.top,
                        u_in: heading(pole, -arriving, index)?,
                        u_out: heading(pole, leaving, index)?,
                    });
                }
                // The loop is not closed there.
                _ => return Err(invalid),
            }
        }
    }
    Ok(Boundary {
        samples,
        passages,
        collapsed,
    })
}

/// `∬ f du dv` over a whole closed surface.
fn whole_surface(surface: &Surface, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Err(MassError::UnboundedFace { face: index });
    }
    const U_PANELS: usize = 8;
    let mut sum = Sums::default();
    let width = (u1 - u0) / U_PANELS as f64;
    for k in 0..U_PANELS {
        let start = u0 + width * k as f64;
        for (u, w) in rule.points(start, start + width) {
            sum.add_scaled(inner(surface, rule, Along::V, u, v0, v1), w);
        }
    }
    Ok(sum)
}

/// `value` moved by whole periods to be as near to `target` as it gets.
fn near(value: f64, target: f64, period: f64) -> f64 {
    value + ((target - value) / period).round() * period
}

/// A face of a surface closed in both directions (see the module
/// documentation): `∬_D f du dv` with `D` oriented by `sense`.
fn doubly_periodic_face(
    face: &Face,
    index: usize,
    rule: &Rule,
    mut samples: Vec<Sample>,
    sense: f64,
    (period_u, period_v): (f64, f64),
) -> Result<Sums, MassError> {
    let surface = &face.surface;
    // Unroll u and v along each loop, and count how often the loop goes
    // around either way before it is back at its first sample.
    let mut around_axis = vec![0.0_f64; face.loops.len()];
    let mut around_tube = vec![0.0_f64; face.loops.len()];
    let mut previous: Option<(usize, f64, f64)> = None;
    let mut first = (0.0, 0.0);
    let mut close = |loop_index: usize, last: (f64, f64), first: (f64, f64)| {
        if let (Some(axis), Some(tube)) = (
            around_axis.get_mut(loop_index),
            around_tube.get_mut(loop_index),
        ) {
            *axis = ((near(first.0, last.0, period_u) - first.0) / period_u).round();
            *tube = ((near(first.1, last.1, period_v) - first.1) / period_v).round();
        }
    };
    for sample in &mut samples {
        match previous {
            Some((loop_index, u, v)) if loop_index == sample.loop_index => {
                sample.u = near(sample.u, u, period_u);
                sample.v = near(sample.v, v, period_v);
            }
            other => {
                if let Some((loop_index, u, v)) = other {
                    close(loop_index, (u, v), first);
                }
                first = (sample.u, sample.v);
            }
        }
        previous = Some((sample.loop_index, sample.u, sample.v));
    }
    if let Some((loop_index, u, v)) = previous {
        close(loop_index, (u, v), first);
    }
    if around_axis.iter().sum::<f64>() != 0.0 || around_tube.iter().sum::<f64>() != 0.0 {
        // Loops that go around without others going back do not enclose
        // a region.
        return Err(MassError::UnboundedFace { face: index });
    }

    let mut sum = Sums::default();
    if around_tube.iter().all(|&turns| turns == 0.0) {
        let v_ref = samples.first().map_or(0.0, |s| s.v);
        for s in &samples {
            if s.du != 0.0 {
                sum.add_scaled(
                    inner(surface, rule, Along::V, s.u, v_ref, s.v),
                    -sense * s.du,
                );
            }
        }
    } else if around_axis.iter().all(|&turns| turns == 0.0) {
        let u_ref = samples.first().map_or(0.0, |s| s.u);
        for s in &samples {
            if s.dv != 0.0 {
                sum.add_scaled(
                    inner(surface, rule, Along::U, s.v, u_ref, s.u),
                    sense * s.dv,
                );
            }
        }
    } else {
        return Err(MassError::Unsupported {
            face: index,
            what: "loops that wind around the axis and around the tube of a torus at once",
        });
    }

    // Right up to a whole number of times the entire surface; the area
    // says which.
    let whole = whole_surface(surface, index, rule)?;
    let share = sum.area / whole.area;
    let times = share.floor();
    if times != 0.0 {
        let rest = share - times;
        if !(rest > 1e-9 && rest < 1.0 - 1e-9) {
            return Err(MassError::Unsupported {
                face: index,
                what: "a face too thin, or too nearly its whole surface, to tell which side of its loops it is on",
            });
        }
        sum.add_scaled(whole, -times);
    }
    Ok(sum)
}

/// `∬_D f du dv` over one face, with `D` oriented by the surface (not by
/// the face's `reversed` flag).
fn face_integrals(body: &Body, face: &Face, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    let surface = &face.surface;
    if face.loops.is_empty() {
        return whole_surface(surface, index, rule);
    }
    let Boundary {
        samples,
        passages,
        collapsed,
    } = boundary(body, face, index, rule)?;
    // Loops run counter-clockwise around the face seen from outside, which
    // is clockwise in parameter space when the face is reversed.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    if let (Some(period_u), Some(period_v)) = (surface.period_u(), surface.period_v()) {
        return doubly_periodic_face(face, index, rule, samples, sense, (period_u, period_v));
    }
    if surface.period_v().is_some() {
        return Err(MassError::Unsupported {
            face: index,
            what: "a surface closed in v only",
        });
    }
    // A collapsed edge of a patch is, like a pole, a stretch of the edge of
    // the parameter range that no edge of the body stands for. Refused
    // until it is added the way poles are.
    if collapsed {
        return Err(MassError::Unsupported {
            face: index,
            what: "a boundary that passes through a collapsed edge of its surface",
        });
    }
    let ((_, _), (v_min, v_max)) = surface.domain();
    let first_v = samples.first().map_or(0.0, |s| s.v);

    let mut stretches = Vec::with_capacity(passages.len());
    for passage in &passages {
        stretches.push(passage.sweep(sense, index)?);
    }
    let mut turns = 0.0;
    let v_ref = match surface.period_u() {
        None => first_v,
        Some(period) => {
            // Net number of times the boundary, with its stretches at the
            // poles, goes around the axis in the surface's
            // counter-clockwise sense.
            let swept = samples.iter().map(|s| s.du).sum::<f64>()
                + stretches.iter().map(|(d, angle)| d * angle).sum::<f64>();
            turns = (sense * swept / period).round();
            if turns == 0.0 {
                // At a pole the boundary passes through, when there is
                // one: what is near it then weighs next to nothing.
                passages.first().map_or(first_v, |p| p.v)
            } else if turns == 1.0 {
                // The face is on the +v side of a loop running in +u.
                v_max
            } else if turns == -1.0 {
                v_min
            } else {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that winds around the axis more than once",
                });
            }
        }
    };
    if !v_ref.is_finite() {
        return Err(MassError::UnboundedFace { face: index });
    }

    let mut sum = Sums::default();
    for s in &samples {
        if s.du != 0.0 {
            sum.add_scaled(
                inner(surface, rule, Along::V, s.u, v_ref, s.v),
                -sense * s.du,
            );
        }
    }
    for (passage, (direction, angle)) in passages.iter().zip(stretches) {
        if passage.v == v_ref {
            continue; // G is zero all along it.
        }
        let count = ((angle / FRAC_PI_4).ceil() as usize).clamp(1, 8);
        let width = angle / count as f64;
        for k in 0..count {
            let start = width * k as f64;
            for (t, w) in rule.points(start, start + width) {
                let u = passage.u_in + direction * t;
                sum.add_scaled(
                    inner(surface, rule, Along::V, u, v_ref, passage.v),
                    -sense * direction * w,
                );
            }
        }
    }
    // A closed surface (a sphere) has an outside of every loop too: the
    // face whose loops circle neither pole but which holds both.
    if turns == 0.0
        && sum.area < 0.0
        && surface.period_u().is_some()
        && let Ok(whole) = whole_surface(surface, index, rule)
    {
        sum.add_scaled(whole, 1.0);
    }
    Ok(sum)
}

/// Area, volume and centroid of `body`. The body should pass
/// [`crate::check`]; on an invalid one the result is meaningless or an
/// error, never a panic.
///
/// Refused with [`MassError::Unsupported`], never guessed: a boundary
/// through a collapsed edge of a Bézier or NURBS surface, a boundary that
/// winds around an axis more than once, torus loops that wind around the
/// axis and the tube at once, a boundary that turns back on itself at a
/// pole or apex.
pub fn mass_properties(body: &Body) -> Result<MassProperties, MassError> {
    let rule = Rule::new(NODES);
    let mut total = Sums::default();
    for (index, face) in body.faces.iter().enumerate() {
        let sums = face_integrals(body, face, index, &rule)?;
        // Area counts whichever way the face points; volume and moments
        // follow the outward normal.
        let outward = if face.reversed { -1.0 } else { 1.0 };
        total.area += sums.area;
        total.volume += sums.volume * outward;
        total.moment += sums.moment * outward;
    }
    if ![total.area, total.volume].iter().all(|x| x.is_finite()) || !total.moment.is_finite() {
        return Err(MassError::Unsupported {
            face: 0,
            what: "geometry whose integrals are not finite",
        });
    }
    let solid = body.kind == BodyKind::Solid;
    Ok(MassProperties {
        area: total.area,
        volume: if solid { total.volume } else { 0.0 },
        centroid: (solid && total.volume != 0.0).then(|| total.moment * (1.0 / total.volume)),
    })
}
