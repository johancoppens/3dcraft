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
//! Four things make `D` more than a region of the plane.
//!
//! **Loops around the axis.** On a surface closed around its axis a loop may
//! wind around it (the rim of a cone, the two ends of a cylinder wall). `G`
//! is periodic in u, so nothing changes, except that a face whose boundary
//! winds once in total reaches to the end of the parameter range on one
//! side (an apex): `v₀` is then that end, where the missing boundary
//! contributes nothing.
//!
//! **The apex of a cone.** At the apex a whole stretch of the edge of the
//! parameter range is one point, and where the boundary passes through it
//! no edge of the body stands for that stretch. With `v₀` at the apex `G`
//! is zero all along it, so nothing is missing; better still, `G du` is
//! then smooth however close to the apex the boundary passes (it is
//! `x dy − y dx` across the axis, times a polynomial), where with any
//! other `v₀` it has a peak as narrow as the boundary is close. So `v₀` is
//! the apex whenever the boundary comes nearer to it than half as far as
//! it reaches from it, and otherwise a point of the boundary itself, so
//! that a band far from the apex is not measured as the difference of two
//! large cones.
//!
//! **The poles of a sphere.** The same holds at the pole `v₀` is put at,
//! but a sphere has two, and no choice of `G` is smooth at both: the area
//! of a sphere is not the boundary integral of anything smooth all over
//! it. A sphere, however, is the same surface whichever way its poles
//! point, and the integrals do not depend on the parametrisation. So a
//! sphere face is measured in a frame of its own, chosen for its boundary
//! and not taken from the surface: the south pole as far from the boundary
//! as it gets (among the axes of the boundary's circles, the direction of
//! its mean point, and the axes and diagonals of the surface's own frame),
//! which puts the north pole, where `v₀` is, in the middle of the
//! boundary. What the boundary integral then gives is the face, less the
//! whole sphere when the face holds the south pole; the area, which lies
//! between zero and the area of the sphere, says which.
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
//! integrals.
//!
//! **Spline surfaces.** A spline surface is smooth only between its knot
//! lines, and so are `f` and `G`. The inner integral is summed stretch by
//! stretch between the surface's knots in its direction, and the boundary
//! panels end where an edge crosses a knot line (found by a change of side
//! along the edge and bisection) as well as at the edge's own knots. `f`
//! is a square root of a polynomial or rational function there, for which
//! no panel count is known to do: the inner panels are doubled along with
//! the boundary's, so the sums below only agree once both have settled. A
//! spline surface may also close on itself around u without saying so (a
//! sphere or a can swept from a rational circle). A boundary that then
//! goes around once in total is the rim of a face that reaches to an end
//! of v: a face only if the surface is a single point there, where `G`
//! then runs from, as from an apex. A face without
//! loops is the whole surface, summed between its knot lines both ways
//! until two sums agree.
//!
//! **Nothing is taken on trust.** The boundary integral of a face is
//! computed twice, first with half the panels its edges need and then with
//! all of them, and again with twice as many until two in a row agree to
//! 1e-12 of the size of what was summed (or to the rounding of the face's
//! coordinates, where that is more); a face for which they never do is an
//! error ([`MassError::Unsupported`]), not a number. Only a plane face
//! bounded by lines, circles and ellipses is exempt. There `G du` is a
//! polynomial of degree four at most, in the parameter of a line or in
//! the sine and cosine of an angle that spans no more than an eighth of a
//! turn per panel; the rule's sixteen nodes are exact for the one and
//! within (π/8 · 4)³² / 32! ≈ 1e-29 of the other. Along a spline the
//! panels end at its knots: it is smooth only between them.

use crate::{Body, BodyKind, Coedge, Face};
use craft3d_geom::{Curve3, GeomError, Sphere, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Vec3};
use std::f64::consts::FRAC_PI_2;
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

    fn parts(self) -> [f64; 5] {
        [
            self.area,
            self.volume,
            self.moment.x,
            self.moment.y,
            self.moment.z,
        ]
    }
}

/// `f(u, v)` for the five integrals, and how large each is before
/// anything cancels: the position times the length of the normal where
/// `f` has their dot product, which is zero all over a cone with its apex
/// at the origin, and the whole normal where it has one component, which
/// is rounding all over a face that a coordinate axis lies in.
fn integrand(surface: &Surface, u: f64, v: f64) -> Total {
    let (s, su, sv) = surface.derivatives(u, v);
    let n = su.cross(sv);
    let sum = Sums {
        area: n.length(),
        volume: s.dot(n) / 3.0,
        moment: Vec3::new(s.x * s.x * n.x, s.y * s.y * n.y, s.z * s.z * n.z) * 0.5,
    };
    Total {
        sum,
        size: Sums {
            area: n.length(),
            volume: s.length() * n.length() / 3.0,
            moment: Vec3::new(s.x * s.x, s.y * s.y, s.z * s.z) * (0.5 * n.length()),
        },
    }
}

const NODES: usize = 16;
/// Panels for an inner integral, at least.
const INNER_PANELS: usize = 2;
/// And at most: an unrolled parameter stays within a few turns.
const MAX_INNER_PANELS: usize = 64;
/// How often the panels along the boundary are doubled, starting from half
/// of what an edge needs, before a face whose sums still change is given
/// up.
const MAX_DOUBLINGS: u32 = 7;
/// Panels along one edge, at most, however often they are doubled.
const MAX_PANELS: usize = 1 << 16;
/// Two sums in a row agree when they differ by no more than this share of
/// the size of what was summed: a thousand times the rounding of the sums,
/// and, the sums converging geometrically, far more than the second of the
/// two is off by.
const CONVERGED: f64 = 1e-12;
/// A face of a closed surface whose area is a smaller share than this of
/// what was summed for it may as well be the whole surface less that: the
/// two differ by the sign of a sum that is zero to rounding.
const TOO_THIN: f64 = 1e-9;
/// An edge that passes this close to an apex, relative to how far the
/// face's boundary reaches from it, passes through it, and is cut there so
/// that the corner it has in parameter space falls between two panels.
/// Taking a near miss for a touch costs nothing: the cut is in the
/// panels, not in what is integrated.
const AT_APEX: f64 = 1e-6;
/// Points along each edge at which a sphere face's boundary is looked at
/// to choose its frame.
const FRAME_SAMPLES: u32 = 32;

/// The direction an inner integral runs in.
#[derive(Clone, Copy)]
enum Along {
    U,
    V,
}

/// A sum, and the sum of the sizes of its terms (see [`integrand`]): what
/// a difference between two sums is measured against.
#[derive(Clone, Copy, Debug, Default)]
struct Total {
    sum: Sums,
    size: Sums,
}

impl Total {
    fn add(&mut self, term: Total, weight: f64) {
        self.sum.add_scaled(term.sum, weight);
        self.size.add_scaled(term.size, weight.abs());
    }

    /// Whether `finer`, the same sum with twice the panels, says the same,
    /// given the share `noise` of every term that is rounding.
    fn agrees_with(&self, finer: &Total, noise: f64) -> bool {
        let size = finer.size.parts();
        self.sum
            .parts()
            .into_iter()
            .zip(finer.sum.parts())
            .zip(size)
            // Not `>`: a NaN agrees with nothing.
            .all(|((a, b), size)| (a - b).abs() <= (CONVERGED + noise) * size)
    }
}

/// `∫ f dt` from `from` to `to` along one parameter, the other one fixed.
///
/// On an analytic surface the integrand is a polynomial or trigonometric
/// and the panels are what makes the rule exact to rounding. On a spline
/// surface it is smooth only between the surface's knots in the direction
/// of `t`, so the panels end at the knots in between, and nothing says how
/// many a stretch needs: half of [`INNER_PANELS`] doubled `doublings`
/// times, along with the panels of the boundary, so that the sums of the
/// boundary only agree once these have settled too.
fn inner(
    surface: &Surface,
    rule: &Rule,
    along: Along,
    fixed: f64,
    (from, to): (f64, f64),
    doublings: u32,
) -> Total {
    let mut sum = Total::default();
    let mut panel = |start: f64, end: f64, sign: f64| {
        for (t, w) in rule.points(start, end) {
            let (u, v) = match along {
                Along::U => (t, fixed),
                Along::V => (fixed, t),
            };
            sum.add(integrand(surface, u, v), w * sign);
        }
    };
    let knots: &[f64] = match (surface, along) {
        (Surface::Nurbs(s), Along::U) => s.knots_u(),
        (Surface::Nurbs(s), Along::V) => s.knots_v(),
        _ => &[],
    };
    if matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        let (low, high, sign) = if from <= to {
            (from, to, 1.0)
        } else {
            (to, from, -1.0)
        };
        let count = ((INNER_PANELS << doublings.min(8)) / 2).clamp(1, MAX_INNER_PANELS);
        let mut stretch = |start: f64, end: f64| {
            let width = (end - start) / count as f64;
            for k in 0..count {
                let at = start + width * k as f64;
                panel(at, at + width, sign);
            }
        };
        let mut start = low;
        for &knot in knots {
            if knot > start && knot < high {
                stretch(start, knot);
                start = knot;
            }
        }
        stretch(start, high);
        return sum;
    }
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
    let width = (to - from) / count as f64;
    for k in 0..count {
        let start = from + width * k as f64;
        panel(start, start + width, 1.0);
    }
    sum
}

/// The quadrature panels along a stretch of an edge, `low` to `high`, as
/// `(start, end)` pairs: half of what the edge needs, doubled `doublings`
/// times.
///
/// What an edge needs: panels of an eighth of a turn at most along a
/// circle or an ellipse, two along a line, and two to every stretch of a
/// spline. A panel ends wherever the integrand has a kink: at the knots
/// of a spline edge, which is smooth only between them, and at `crossings`,
/// where the edge crosses a knot line of a spline surface. A sum over
/// panels that straddle a kink converges too slowly to ever settle.
fn panels(
    curve: &Curve3,
    (low, high): (f64, f64),
    crossings: &[f64],
    doublings: u32,
) -> Vec<(f64, f64)> {
    let double = 1_usize << doublings.min(16);
    let mut stops = vec![low];
    if let Curve3::Nurbs(c) = curve {
        stops.extend(c.knots().iter().filter(|&&k| k > low && k < high));
    }
    stops.extend(crossings.iter().filter(|&&t| t > low && t < high));
    stops.push(high);
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    let stretches = stops.len().saturating_sub(1).max(1);
    let mut panels = Vec::new();
    for pair in stops.windows(2) {
        let &[from, to] = pair else { continue };
        let needed = match curve {
            Curve3::Line(_) | Curve3::Nurbs(_) => 1,
            Curve3::Circle(_) | Curve3::Ellipse(_) => {
                (((to - from) / FRAC_PI_2).ceil() as usize).clamp(1, 32)
            }
        };
        let count = needed
            .saturating_mul(double)
            .min(MAX_PANELS / stretches)
            .max(1);
        let width = (to - from) / count as f64;
        for k in 0..count {
            let start = from + width * k as f64;
            panels.push((start, start + width));
        }
    }
    panels
}

/// The distinct knots of a spline surface strictly inside its domain, in
/// u and in v: its knot lines, across which it is not smooth.
fn knot_lines(surface: &Surface) -> (Vec<f64>, Vec<f64>) {
    let Surface::Nurbs(s) = surface else {
        return (Vec::new(), Vec::new());
    };
    let ((u0, u1), (v0, v1)) = s.domain();
    let inside = |knots: &[f64], (start, end): (f64, f64)| {
        let mut lines: Vec<f64> = knots
            .iter()
            .copied()
            .filter(|&k| k > start && k < end)
            .collect();
        lines.dedup();
        lines
    };
    (inside(s.knots_u(), (u0, u1)), inside(s.knots_v(), (v0, v1)))
}

/// Looks along an edge at this many places per stretch between its knots
/// for crossings of the surface's knot lines.
const CROSSING_STEPS: u32 = 8;

/// Where the stretch `low` to `high` of an edge crosses a knot line of
/// the spline surface it lies on (or the seam of one that closes on
/// itself): parameters of the edge, in no particular order.
///
/// Found by a change of side between neighbouring places along the edge
/// and narrowed by bisection. An edge that runs along a knot line does
/// not cross it; one that touches a line and turns back between two of
/// the places looked at is missed, and its sums then do not settle.
fn knot_crossings(
    curve: &Curve3,
    surface: &Surface,
    (low, high): (f64, f64),
) -> Result<Vec<f64>, GeomError> {
    let (lines_u, lines_v) = knot_lines(surface);
    let mut crossings = Vec::new();
    if (lines_u.is_empty() && lines_v.is_empty()) || high <= low || (high - low).is_nan() {
        return Ok(crossings);
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    let at = |t: f64| surface.project(curve.eval(t));
    // The places to look at: evenly between the edge's own knots.
    let mut places = vec![low];
    let mut ends = vec![low];
    if let Curve3::Nurbs(c) = curve {
        ends.extend(c.knots().iter().filter(|&&k| k > low && k < high));
    }
    ends.push(high);
    ends.dedup();
    for pair in ends.windows(2) {
        let &[from, to] = pair else { continue };
        for k in 1..=CROSSING_STEPS {
            places.push(from + (to - from) * f64::from(k) / f64::from(CROSSING_STEPS));
        }
    }
    let mut before: Option<(f64, (f64, f64))> = None;
    for t in places {
        let here = at(t)?;
        if let Some((s, there)) = before {
            for (lines, from, to, width, pick_u) in [
                (&lines_u, there.0, here.0, u1 - u0, true),
                (&lines_v, there.1, here.1, v1 - v0, false),
            ] {
                let pick = |uv: (f64, f64)| if pick_u { uv.0 } else { uv.1 };
                // The side of `x` the stretch started on, for bisection.
                let mut narrow = |starts: &dyn Fn(f64) -> bool| -> Result<(), GeomError> {
                    let (mut a, mut b) = (s, t);
                    for _ in 0..48 {
                        let middle = 0.5 * (a + b);
                        if !(middle > a && middle < b) {
                            break;
                        }
                        if starts(pick(at(middle)?)) {
                            a = middle;
                        } else {
                            b = middle;
                        }
                    }
                    crossings.push(0.5 * (a + b));
                    Ok(())
                };
                if (to - from).abs() > 0.5 * width {
                    // From one side of a seam to the other.
                    narrow(&|x| (x - from).abs() < (x - to).abs())?;
                    continue;
                }
                // On a line (within rounding) is on neither side of it.
                let clear = 1e-9 * width;
                for &line in lines.iter() {
                    let (d0, d1) = (from - line, to - line);
                    if (d0 > clear && d1 < -clear) || (d0 < -clear && d1 > clear) {
                        narrow(&|x| (x - line > 0.0) == (d0 > 0.0))?;
                    }
                }
            }
        }
        before = Some((t, here));
    }
    Ok(crossings)
}

/// The frame a sphere face is measured in (see the module documentation):
/// the same sphere with its south pole as far from the face's boundary as
/// the candidates allow. `None` when the boundary has no finite point.
fn own_sphere(body: &Body, face: &Face, sphere: &Sphere) -> Option<Surface> {
    let (old, radius) = (*sphere.frame(), sphere.radius());
    let centre = old.origin();
    let mut points = Vec::new();
    let mut candidates: Vec<Vec3> = Vec::new();
    let mut circles = Vec::new();
    for loop_id in &face.loops {
        for coedge in &body.loop_(*loop_id)?.coedges {
            let edge = body.edge(coedge.edge)?;
            let (t0, t1) = edge.range;
            for k in 0..=FRAME_SAMPLES {
                let point = edge
                    .curve
                    .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(FRAME_SAMPLES));
                if point.is_finite() {
                    points.push(point);
                }
            }
            if let Curve3::Circle(circle) = &edge.curve {
                circles.push(circle.frame().z());
            }
        }
    }
    // The middle of the boundary, when it has one.
    let mut mean = Vec3::ZERO;
    for point in &points {
        mean += (*point - centre) * (1.0 / points.len() as f64);
    }
    if mean.length() > 1e-3 * radius
        && let Some(direction) = mean.normalized()
    {
        candidates.push(direction);
    }
    candidates.extend(circles);
    candidates.extend([old.z(), old.x(), old.y()]);
    let diagonal = 1.0 / 3.0_f64.sqrt();
    for (x, y) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        candidates.push(old.vector(x, y, 1.0) * diagonal);
    }
    let mut best: Option<(f64, Vec3)> = None;
    for candidate in candidates {
        for north in [candidate, -candidate] {
            let south = centre - north * radius;
            let clear = points
                .iter()
                .map(|point| point.distance(south))
                .fold(f64::INFINITY, f64::min);
            // The first of equals: the order above is part of the result.
            if clear.is_finite() && best.is_none_or(|(most, _)| clear > most) {
                best = Some((clear, north));
            }
        }
    }
    let frame = Frame::from_z(centre, best?.1)?;
    Some(Surface::Sphere(Sphere::new(frame, radius).ok()?))
}

/// A stretch of an edge as a loop runs along it, from parameter `start` to
/// `end` (either may be the larger).
struct Piece<'a> {
    curve: &'a Curve3,
    start: f64,
    end: f64,
    /// Where it crosses a knot line of its spline surface.
    crossings: Vec<f64>,
}

/// The boundary of a face: per loop its pieces in the order the loop runs.
struct Outline<'a> {
    loops: Vec<Vec<Piece<'a>>>,
    /// Whether the boundary touches a point where a freeform surface's
    /// parametrisation collapses (a collapsed patch edge).
    collapsed: bool,
    /// Whether every edge is a line, a circle or an ellipse.
    conic: bool,
    /// The share of the boundary's extent that is the rounding of its
    /// coordinates: four units in the last place of the largest of them,
    /// over how far the boundary reaches across. A face a million times
    /// smaller than it is far from the origin is known to ten digits, and
    /// its sums cannot agree to more.
    noise: f64,
}

fn outline<'a>(
    body: &'a Body,
    face: &Face,
    surface: &Surface,
    index: usize,
) -> Result<Outline<'a>, MassError> {
    let invalid = MassError::Invalid { face: index };
    let geometry = |error| MassError::Geometry { face: index, error };
    let apex = match surface {
        Surface::Cone(c) => {
            let v = c.apex_v();
            v.is_finite().then(|| c.frame().point(0.0, 0.0, v))
        }
        _ => None,
    };
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));

    // How far the boundary reaches from the apex, and whether it touches a
    // collapsed edge of a freeform surface.
    let mut reach = 0.0_f64;
    let mut extent = Bbox3::EMPTY;
    let mut furthest = 0.0_f64;
    let mut collapsed = false;
    let mut conic = true;
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_id in &face.loops {
        let loop_ = body.loop_(*loop_id).ok_or(invalid.clone())?;
        for coedge in &loop_.coedges {
            let edge = body.edge(coedge.edge).ok_or(invalid.clone())?;
            if body.vertex(edge.start).is_none() || body.vertex(edge.end).is_none() {
                return Err(invalid);
            }
            conic &= !matches!(edge.curve, Curve3::Nurbs(_));
            let (t0, t1) = edge.range;
            for t in [t0, 0.5 * (t0 + t1), t1] {
                let point = edge.curve.eval(t);
                if point.is_finite() {
                    extent.add(point);
                    furthest = furthest.max(point.length());
                }
                if let Some(apex) = apex {
                    let distance = point.distance(apex);
                    if distance.is_finite() {
                        reach = reach.max(distance);
                    }
                }
            }
            if freeform {
                for end in [t0, t1] {
                    let (u, v) = surface.project(edge.curve.eval(end)).map_err(geometry)?;
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

    let mut pieces = Vec::with_capacity(loops.len());
    for coedges in loops {
        let mut run = Vec::with_capacity(coedges.len());
        for &Coedge { edge, reversed } in coedges {
            let edge = body.edge(edge).ok_or(invalid.clone())?;
            let (t0, t1) = edge.range;
            // Where the edge passes through the apex on its way.
            let mut cut = None;
            if let Some(apex) = apex
                && t1 > t0
            {
                let mut t = edge.curve.project(apex).map_err(geometry)?;
                if let Some(period) = edge.curve.period() {
                    t = t0 + (t - t0).rem_euclid(period);
                }
                let margin = 1e-9 * (t1 - t0);
                if t > t0 + margin
                    && t < t1 - margin
                    && edge.curve.eval(t).distance(apex) <= AT_APEX * reach
                {
                    cut = Some(t);
                }
            }
            let curve = &edge.curve;
            let crossings = knot_crossings(curve, surface, (t0, t1)).map_err(geometry)?;
            let mut stretch = |start: f64, end: f64| {
                let crossings = crossings.clone();
                run.push(if reversed {
                    Piece {
                        curve,
                        start: end,
                        end: start,
                        crossings,
                    }
                } else {
                    Piece {
                        curve,
                        start,
                        end,
                        crossings,
                    }
                });
            };
            match (cut, reversed) {
                (None, _) => stretch(t0, t1),
                (Some(t), false) => {
                    stretch(t0, t);
                    stretch(t, t1);
                }
                (Some(t), true) => {
                    stretch(t, t1);
                    stretch(t0, t);
                }
            }
        }
        pieces.push(run);
    }
    let across = extent.diagonal();
    let noise = if across > 0.0 {
        (4.0 * f64::EPSILON * furthest / across).min(1.0)
    } else {
        // Smaller than its coordinates resolve: any sum is as good.
        1.0
    };
    Ok(Outline {
        loops: pieces,
        collapsed,
        conic,
        noise,
    })
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

/// The boundary as quadrature samples, in the order the loops run, with
/// half the panels every edge needs doubled `doublings` times.
fn samples(
    outline: &Outline,
    surface: &Surface,
    index: usize,
    rule: &Rule,
    doublings: u32,
) -> Result<Vec<Sample>, MassError> {
    let mut samples = Vec::new();
    for (loop_index, pieces) in outline.loops.iter().enumerate() {
        for piece in pieces {
            let (low, high) = (piece.start.min(piece.end), piece.start.max(piece.end));
            let direction = if piece.end < piece.start { -1.0 } else { 1.0 };
            let first = samples.len();
            for (start, end) in panels(piece.curve, (low, high), &piece.crossings, doublings) {
                for (t, w) in rule.points(start, end) {
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
                    // At a pole or an apex Sᵤ vanishes and u has no rate of
                    // change; `G` is zero there or the boundary is not.
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
        }
    }
    Ok(samples)
}

/// `∬ f du dv` over a whole closed surface.
///
/// On a spline surface the panels in u end at its knots, like those in v
/// ([`inner`]), and are doubled until two sums agree.
fn whole_surface(surface: &Surface, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Err(MassError::UnboundedFace { face: index });
    }
    let across = |from: f64, to: f64, count: usize, doublings: u32, sum: &mut Total| {
        let width = (to - from) / count as f64;
        for k in 0..count {
            let start = from + width * k as f64;
            for (u, w) in rule.points(start, start + width) {
                sum.add(inner(surface, rule, Along::V, u, (v0, v1), doublings), w);
            }
        }
    };
    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        const U_PANELS: usize = 8;
        let mut sum = Total::default();
        across(u0, u1, U_PANELS, 0, &mut sum);
        return Ok(sum.sum);
    }
    let (mut lines, _) = knot_lines(surface);
    lines.push(u1);
    let mut coarser: Option<Total> = None;
    for doublings in 0..=MAX_DOUBLINGS {
        let mut sum = Total::default();
        let mut start = u0;
        for &line in &lines {
            across(start, line, 1 << doublings, doublings, &mut sum);
            start = line;
        }
        if coarser.is_some_and(|coarser| coarser.agrees_with(&sum, 0.0)) {
            return Ok(sum.sum);
        }
        coarser = Some(sum);
    }
    Err(MassError::Unsupported {
        face: index,
        what: "an integral over a whole spline surface that does not settle",
    })
}

/// `value` moved by whole periods to be as near to `target` as it gets.
fn near(value: f64, target: f64, period: f64) -> f64 {
    value + ((target - value) / period).round() * period
}

/// How the boundary integral of a face is set up. Decided once, from the
/// coarsest samples, so that the finer sums are sums of the same thing.
enum Plan {
    /// `G` runs from this v.
    From(f64),
    /// A surface closed both ways: the parameters each loop's unrolling
    /// starts near, and the direction the inner integral runs in.
    Unrolled {
        starts: Vec<(f64, f64)>,
        along: Along,
    },
}

/// Unrolls u and v along each loop of a surface closed both ways, each
/// loop starting near its entry in `starts` (or wherever its first sample
/// is). Gives where each loop started and how often it goes around the
/// axis and around the tube before it is back at its first sample.
fn unroll(
    samples: &mut [Sample],
    loops: usize,
    starts: Option<&[(f64, f64)]>,
    (period_u, period_v): (f64, f64),
) -> (Vec<(f64, f64)>, Vec<f64>, Vec<f64>) {
    let mut began = vec![(0.0, 0.0); loops];
    let mut around_axis = vec![0.0_f64; loops];
    let mut around_tube = vec![0.0_f64; loops];
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
    for sample in samples.iter_mut() {
        match previous {
            Some((loop_index, u, v)) if loop_index == sample.loop_index => {
                sample.u = near(sample.u, u, period_u);
                sample.v = near(sample.v, v, period_v);
            }
            other => {
                if let Some((loop_index, u, v)) = other {
                    close(loop_index, (u, v), first);
                }
                if let Some(&(u, v)) = starts.and_then(|s| s.get(sample.loop_index)) {
                    sample.u = near(sample.u, u, period_u);
                    sample.v = near(sample.v, v, period_v);
                }
                first = (sample.u, sample.v);
                if let Some(began) = began.get_mut(sample.loop_index) {
                    *began = first;
                }
            }
        }
        previous = Some((sample.loop_index, sample.u, sample.v));
    }
    if let Some((loop_index, u, v)) = previous {
        close(loop_index, (u, v), first);
    }
    (began, around_axis, around_tube)
}

/// The plan for a face from its coarsest samples (see the module
/// documentation), which are unrolled on the way for a surface closed
/// both ways.
fn plan(
    face: &Face,
    surface: &Surface,
    index: usize,
    samples: &mut [Sample],
    sense: f64,
) -> Result<Plan, MassError> {
    if let (Some(period_u), Some(period_v)) = (surface.period_u(), surface.period_v()) {
        let (starts, around_axis, around_tube) =
            unroll(samples, face.loops.len(), None, (period_u, period_v));
        if around_axis.iter().sum::<f64>() != 0.0 || around_tube.iter().sum::<f64>() != 0.0 {
            // Loops that go around without others going back do not enclose
            // a region.
            return Err(MassError::UnboundedFace { face: index });
        }
        let along = if around_tube.iter().all(|&turns| turns == 0.0) {
            Along::V
        } else if around_axis.iter().all(|&turns| turns == 0.0) {
            Along::U
        } else {
            return Err(MassError::Unsupported {
                face: index,
                what: "loops that wind around the axis and around the tube of a torus at once",
            });
        };
        return Ok(Plan::Unrolled { starts, along });
    }
    if surface.period_v().is_some() {
        return Err(MassError::Unsupported {
            face: index,
            what: "a surface closed in v only",
        });
    }
    let ((u_min, u_max), (v_min, v_max)) = surface.domain();
    let first_v = samples.first().map_or(0.0, |s| s.v);
    let v_ref = match (surface, surface.period_u()) {
        // The north pole of the frame chosen for the face.
        (Surface::Sphere(_), _) => v_max,
        (Surface::Plane(_), _) => first_v,
        (_, None) => {
            // A spline surface may close on itself around u without
            // saying so, and a boundary that goes around it once in total
            // (the rim of a cap on a sphere of revolution) is not closed
            // in its parameters: the face reaches to the end of v on one
            // side. That is a face only if the surface is a single point
            // there, a pole, which is then where `G` runs from, as from
            // an apex.
            let swept = samples.iter().map(|s| s.du).sum::<f64>();
            let turns = (sense * swept / (u_max - u_min)).round();
            let risen = samples.iter().map(|s| s.dv).sum::<f64>();
            if (risen / (v_max - v_min)).round() != 0.0 {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that goes around a spline surface in v",
                });
            }
            if turns == 0.0 {
                first_v
            } else if turns.abs() == 1.0 {
                let end = if turns > 0.0 { v_max } else { v_min };
                let pole = (0..=8).all(|k| {
                    let u = u_min + (u_max - u_min) * f64::from(k) / 8.0;
                    let (_, su, sv) = surface.derivatives(u, end);
                    su.length() <= 1e-9 * (su.length() + sv.length())
                });
                if !pole {
                    return Err(MassError::UnboundedFace { face: index });
                }
                end
            } else {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that winds around the axis more than once",
                });
            }
        }
        (_, Some(period)) => {
            let apex = match surface {
                Surface::Cone(c) => Some(c.apex_v()).filter(|v| v.is_finite()),
                _ => None,
            };
            let near_apex = apex.filter(|&apex| {
                let (mut nearest, mut furthest) = (f64::INFINITY, 0.0_f64);
                for s in samples.iter() {
                    nearest = nearest.min((s.v - apex).abs());
                    furthest = furthest.max((s.v - apex).abs());
                }
                nearest < 0.5 * furthest
            });
            if let Some(apex) = near_apex {
                apex
            } else {
                // Net number of times the boundary goes around the axis in
                // the surface's counter-clockwise sense. It stays well
                // away from an apex here, so the sum is a good one.
                let swept = samples.iter().map(|s| s.du).sum::<f64>();
                let turns = (sense * swept / period).round();
                if turns == 0.0 {
                    first_v
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
        }
    };
    if !v_ref.is_finite() {
        return Err(MassError::UnboundedFace { face: index });
    }
    Ok(Plan::From(v_ref))
}

/// The boundary integral `-∮ G du` (or `∮ H dv`) of a face by `plan`.
fn boundary_integral(
    face: &Face,
    surface: &Surface,
    rule: &Rule,
    samples: &mut [Sample],
    sense: f64,
    plan: &Plan,
    (unrolled, doublings): (bool, u32),
) -> Total {
    let mut total = Total::default();
    match plan {
        Plan::From(v_ref) => {
            for s in samples.iter() {
                if s.du != 0.0 {
                    total.add(
                        inner(surface, rule, Along::V, s.u, (*v_ref, s.v), doublings),
                        -sense * s.du,
                    );
                }
            }
        }
        Plan::Unrolled { starts, along } => {
            if !unrolled
                && let (Some(period_u), Some(period_v)) = (surface.period_u(), surface.period_v())
            {
                unroll(
                    samples,
                    face.loops.len(),
                    Some(starts),
                    (period_u, period_v),
                );
            }
            let (u_ref, v_ref) = starts.first().copied().unwrap_or((0.0, 0.0));
            for s in samples.iter() {
                match along {
                    Along::V if s.du != 0.0 => total.add(
                        inner(surface, rule, Along::V, s.u, (v_ref, s.v), doublings),
                        -sense * s.du,
                    ),
                    Along::U if s.dv != 0.0 => total.add(
                        inner(surface, rule, Along::U, s.v, (u_ref, s.u), doublings),
                        sense * s.dv,
                    ),
                    _ => {}
                }
            }
        }
    }
    total
}

/// What a converged boundary integral says about the face.
fn settle(surface: &Surface, index: usize, rule: &Rule, total: Total) -> Result<Sums, MassError> {
    let mut sum = total.sum;
    let closed_both_ways = surface.period_u().is_some() && surface.period_v().is_some();
    if closed_both_ways {
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
    } else if matches!(surface, Surface::Sphere(_)) {
        // The face, less the whole sphere when the face holds the south
        // pole; the sign of the area says which, unless the sum is zero
        // to rounding.
        if sum.area.abs() <= TOO_THIN * total.size.area {
            return Err(MassError::Unsupported {
                face: index,
                what: "a boundary that doubles back on itself, or a face too thin, or too nearly its whole sphere, to tell which side of its loops it is on",
            });
        }
        if sum.area < 0.0 {
            sum.add_scaled(whole_surface(surface, index, rule)?, 1.0);
        }
    } else if sum.area < 0.0 {
        // The loops run the wrong way around what they enclose: the face
        // is everything outside them.
        return Err(MassError::UnboundedFace { face: index });
    }
    Ok(sum)
}

/// `∬_D f du dv` over one face, with `D` oriented by the surface (not by
/// the face's `reversed` flag).
fn face_integrals(body: &Body, face: &Face, index: usize, rule: &Rule) -> Result<Sums, MassError> {
    if face.loops.is_empty() {
        return whole_surface(&face.surface, index, rule);
    }
    let own = match &face.surface {
        Surface::Sphere(sphere) => own_sphere(body, face, sphere),
        _ => None,
    };
    let surface = own.as_ref().unwrap_or(&face.surface);
    integrals_in(body, face, surface, index, rule)
}

/// [`face_integrals`] of a face with loops, with `surface` standing in for
/// the face's own: the same surface, parametrised to suit the face.
fn integrals_in(
    body: &Body,
    face: &Face,
    surface: &Surface,
    index: usize,
    rule: &Rule,
) -> Result<Sums, MassError> {
    let outline = outline(body, face, surface, index)?;
    // Loops run counter-clockwise around the face seen from outside, which
    // is clockwise in parameter space when the face is reversed.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    // Exact with the panels an edge needs (see the module documentation).
    let exact = outline.conic && matches!(surface, Surface::Plane(_));

    let mut set_up: Option<Plan> = None;
    let mut coarser: Option<Total> = None;
    for doublings in u32::from(exact)..=MAX_DOUBLINGS {
        let mut samples = samples(&outline, surface, index, rule, doublings)?;
        let (plan, unrolled) = match set_up.take() {
            Some(plan) => (plan, false),
            None => {
                let plan = plan(face, surface, index, &mut samples, sense)?;
                // A collapsed edge of a patch is, like an apex, a stretch
                // of the edge of the parameter range that no edge of the
                // body stands for. Refused until it is added.
                if outline.collapsed {
                    return Err(MassError::Unsupported {
                        face: index,
                        what: "a boundary that passes through a collapsed edge of its surface",
                    });
                }
                (plan, true)
            }
        };
        let total = boundary_integral(
            face,
            surface,
            rule,
            &mut samples,
            sense,
            &plan,
            (unrolled, doublings),
        );
        if exact || coarser.is_some_and(|coarser| coarser.agrees_with(&total, outline.noise)) {
            return settle(surface, index, rule, total);
        }
        coarser = Some(total);
        set_up = Some(plan);
    }
    Err(MassError::Unsupported {
        face: index,
        what: "a boundary integral that does not settle: the boundary is too intricate, or too close to where the surface's parametrisation collapses",
    })
}

/// Area, volume and centroid of `body`. The body should pass
/// [`crate::check`]; on an invalid one the result is meaningless or an
/// error, never a panic.
///
/// Refused with [`MassError::Unsupported`], never guessed: a boundary
/// through a collapsed edge of a Bézier or NURBS surface, a boundary that
/// winds around an axis more than once, torus loops that wind around the
/// axis and the tube at once, a face of a sphere or a torus too thin (or
/// too nearly the whole surface) to tell which side of its loops it is
/// on, and a face whose boundary integral does not settle. A face that is
/// outside its own loops is [`MassError::UnboundedFace`].
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

#[cfg(test)]
mod tests {
    //! The parts of the integrator that no body reaches from outside,
    //! because it chooses a sphere face's frame itself: what a frame does
    //! that it would not choose.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::unreachable
    )]

    use super::*;
    use crate::{Body, BodyKind, Coedge};
    use craft3d_geom::Circle;
    use std::f64::consts::{PI, TAU};

    const RADIUS: f64 = 2.0;
    const CENTRE: Vec3 = Vec3::new(0.3, -1.1, 0.7);

    /// A cap of the sphere about `CENTRE` beyond the plane `level` along
    /// z, as a sheet of one face; its area is 2πr (r − level).
    fn cap(level: f64) -> Body {
        let mut body = Body::new(BodyKind::Sheet);
        let rim = (RADIUS * RADIUS - level * level).sqrt();
        let frame = Frame::WORLD
            .at(CENTRE + Vec3::new(0.0, 0.0, level))
            .unwrap();
        let vertex = body.add_vertex(frame.point(rim, 0.0, 0.0));
        let edge = body.add_edge(
            Curve3::Circle(Circle::new(frame, rim).unwrap()),
            (0.0, TAU),
            vertex,
            vertex,
        );
        let loop_ = body.add_loop(vec![Coedge {
            edge,
            reversed: false,
        }]);
        let sphere = Sphere::new(Frame::WORLD.at(CENTRE).unwrap(), RADIUS).unwrap();
        let face = body.add_face(Surface::Sphere(sphere), false, vec![loop_]);
        body.add_shell(vec![face]);
        body
    }

    /// The cap's sphere with its north pole `tilt` away from the cap's
    /// middle.
    fn turned(tilt: f64) -> Surface {
        let pole = Vec3::new(tilt.sin() * 0.6, tilt.sin() * 0.8, tilt.cos());
        let frame = Frame::new(CENTRE, pole, Vec3::new(0.8, -0.6, 0.0)).unwrap();
        Surface::Sphere(Sphere::new(frame, RADIUS).unwrap())
    }

    fn area_in(body: &Body, surface: &Surface) -> Result<f64, MassError> {
        let rule = Rule::new(NODES);
        let face = body.faces.first().unwrap();
        integrals_in(body, face, surface, 0, &rule).map(|sums| sums.area)
    }

    /// The pole `G` runs from may be anywhere: in the face, outside it,
    /// on its boundary or a hair from it. `G du` is smooth there.
    #[test]
    fn the_north_pole_may_be_on_the_boundary() {
        for level in [-1.5_f64, 0.0, 1.2] {
            let body = cap(level);
            let expected = TAU * RADIUS * (RADIUS - level);
            let rim = (level / RADIUS).acos();
            let mut tilts = vec![0.0, rim / 2.0, rim, (rim + PI) / 2.0];
            for off in [1e-13, 1e-10, 1e-7, 1e-5, 1e-3, 0.1] {
                tilts.extend([rim - off, rim + off]);
            }
            for tilt in tilts {
                // The other pole stays clear of the boundary by 0.2.
                if (PI - tilt - rim).abs() < 0.2 {
                    continue;
                }
                let area = area_in(&body, &turned(tilt)).unwrap();
                assert!(
                    (area - expected).abs() <= 1e-11 * expected,
                    "level {level}, pole {tilt}: {area} vs {expected}"
                );
            }
        }
    }

    /// The other pole may not: no `G` is smooth at both. A boundary that
    /// passes close to it takes more panels, and one that passes through
    /// it, or closer than the panels can be made fine, is refused. What
    /// is never returned is a number that is not the area.
    #[test]
    fn the_south_pole_on_the_boundary_is_refused_not_guessed() {
        let level = 0.8_f64;
        let body = cap(level);
        let expected = TAU * RADIUS * (RADIUS - level);
        let rim = (level / RADIUS).acos();
        let mut refused = 0;
        let mut measured = 0;
        for off in [
            0.0, 1e-13, -1e-13, 1e-9, -1e-9, 1e-6, -1e-6, 1e-4, -1e-4, 1e-3, -1e-3, 1e-2, -1e-2,
            0.1, -0.1, 0.5, -0.5,
        ] {
            // The south pole `off` outside the rim (inside, if negative).
            match area_in(&body, &turned(PI - rim - off)) {
                Ok(area) => {
                    measured += 1;
                    assert!(
                        (area - expected).abs() <= 1e-11 * expected,
                        "south pole {off} from the rim: {area} vs {expected}"
                    );
                }
                Err(MassError::Unsupported { what, .. }) => {
                    refused += 1;
                    assert!(what.contains("does not settle"), "{what}");
                    // The finest two sums have panels 0.025 and 0.012 wide
                    // and must agree: that takes more room than 0.01.
                    assert!(off.abs() <= 1e-2, "refused {off} from the rim");
                }
                Err(other) => panic!("south pole {off} from the rim: {other}"),
            }
        }
        // Through the pole it cannot be measured; a tenth away it can.
        assert!(
            refused >= 1 && measured >= 4,
            "{refused} refused, {measured} measured"
        );
    }

    /// The frame chosen for a face keeps its south pole away from the
    /// boundary: opposite the middle of a cap, whatever frame the sphere
    /// came with.
    #[test]
    fn the_chosen_frame_has_its_south_pole_clear_of_the_boundary() {
        for level in [-1.9_f64, -0.5, 0.0, 0.5, 1.9] {
            for tilt in [0.0, 0.4, (level / RADIUS).acos(), 2.0, PI] {
                let mut body = cap(level);
                body.faces[0].surface = turned(tilt);
                let Surface::Sphere(given) = &body.faces[0].surface else {
                    unreachable!()
                };
                let Some(Surface::Sphere(own)) = own_sphere(&body, &body.faces[0], given) else {
                    panic!("no frame for level {level}");
                };
                // North is the pole nearer the rim: +z above the equator.
                let north = own.frame().z();
                let expected = if level >= 0.0 { 1.0 } else { -1.0 };
                assert!(
                    (north.z - expected).abs() < 1e-12,
                    "level {level}, tilt {tilt}: north {north:?}"
                );
                assert_eq!(own.frame().origin(), CENTRE);
                assert_eq!(own.radius(), RADIUS);
            }
        }
    }
}
