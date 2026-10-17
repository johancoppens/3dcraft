//! Blends whose face has no closed form: a fillet along an edge between
//! faces that are not a plane and a cylinder along a straight edge or
//! surfaces of revolution about a circle's axis (a straight edge between
//! a plane and a cone, an edge of a lofted blade, the curve where a blade
//! meets a hub), and a chamfer along such an edge.
//!
//! **The face.** A fillet is the surface a ball of the radius sweeps as it
//! rolls along the edge touching both faces: the balls' centres (the
//! spine) are the points the radius away from both surfaces, on the
//! material's side at a convex edge and on the other at a concave one.
//! Each ball is found in a plane across the edge (through a point of the
//! edge, square to it; past the ends of the edge along its tangent
//! there), by moving its centre to where the two tangent planes at the
//! nearest points of the surfaces, moved by the radius, cross that plane,
//! until it stops moving. The face meets each ball in the short arc
//! between the two points where the ball touches the faces, a circle of
//! the radius in the plane of the centre and those two points. Which
//! plane across the edge picks a ball does not matter for the surface:
//! the balls that touch both faces are one family, and the planes only
//! choose which of them are sampled. A chamfer is, in each such plane,
//! the line between the two points of the faces the distances away from
//! the edge's point (a straight distance, measured in the plane).
//!
//! **The approximation.** Each arc is a rational quadratic, exact; the
//! arcs (or the chamfer lines) of a row of planes are joined into one
//! B-spline surface by interpolating their homogeneous control points
//! with a cubic, so the surface passes through every sampled section
//! exactly. Between two samples it is held against the exact section
//! there, point for point in the same parameter, and samples are added
//! until every such distance is below a quarter of the tolerance: that
//! bounds the distance to the true surface from above. The two borders of
//! the surface (where it touches the faces) are its own iso-curves, so the
//! face and the faces beside it share their edges exactly; each is proven
//! within a quarter of the tolerance of its face between the samples.
//!
//! **Where it ends.** An edge that ends in a vertex of exactly three
//! faces (the two of the edge and a third one across it): the blend face
//! is continued past the vertex until it is wholly out of the third face,
//! and cut where it crosses that face's surface, along a curve fitted to
//! points found exactly (and proven within a quarter of the tolerance of
//! both surfaces between them, `geom::certify`). The two other edges at the vertex are shortened to where the
//! borders of the blend reach them, the third face gets the cut curve in
//! place of the corner, and the edge becomes the blend face.
//!
//! **A closed edge** (one vertex no other edge passes through) has no
//! ends: the sections go round it (a fillet's in the planes square to the
//! closed path of the balls' centres, which turn smoothly where the edge
//! is a fitted curve whose planes would not), the surface is the closed
//! cubic through them, and the blend face is the ring between its two
//! borders.
//!
//! **Chains and meets.** Edges that go on smoothly into each other are
//! blended as one chain, and two fillets that end in one vertex at an
//! angle meet there: see [`chain`].
//!
//! **What is refused** (with the reason): an edge that ends in a vertex of
//! more than three faces, or where blended edges meet in any other way
//! than a chain or a meet; a closed edge whose vertex other edges pass
//! through; a chamfer given by an angle; a blend whose borders leave the
//! edges at its ends (too large), or run off a face beside the edge along
//! the way; a surface that cannot be followed (a ball that does not
//! settle, an arc of half a turn or more); an approximation that does not
//! come within the tolerance in the samples allowed. The body is changed in place (every other entity
//! keeps its number), passes `craft3d_topo::check` and the tangency test
//! of [`super`]; whether a large blend runs into a face further away is
//! not seen here: the caller looks for places where the body passes
//! through itself, as for a loft.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{BlendError, Blended, Face, Reason, Request, Size, lost, refused};
use crate::KernelError;
use crate::chamfer::ChamferSize;
use craft3d_geom::certify::{Apart, CurveBound, SurfaceBound, curves_apart};
use craft3d_geom::refine::interpolate;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Edge, EdgeId, FaceId, Loop, LoopId, Vertex, VertexId};

mod chain;
mod cross;
mod exact;

/// How many times a step between two sections may be halved to follow
/// the ball from one to the other.
const FOLLOW_DEPTH: u32 = 8;

/// Most samples along one blend face, and across one cut at its end.
const MAX_SAMPLES: usize = 4096;
/// Passes of the refinement: each halves the steps where the surface
/// does not hold.
const PASSES: usize = 16;
/// Steps of the search for one ball, and of the search for a nearest point.
const BALL_STEPS: usize = 100;
const FOOT_STEPS: usize = 60;
/// How far a fillet face may turn away from the faces it touches, at
/// least: what the approximation is refined to between samples. It may
/// turn by the document's angular tolerance, or by the angle the linear
/// tolerance makes over the radius, whichever is more: a surface within
/// the linear tolerance of the true one can lean by that much.
const BEND: f64 = 1e-9;

/// The sine to which a blend's normals are refined at `size` (its radius
/// or largest distance): the angular tolerance, [`BEND`] at least, or the
/// angle the linear tolerance makes over the size, whichever is more. The
/// tangency of a fillet along its borders, across a joint and across a
/// crossed smooth edge is measured to ten times it.
fn bend(tolerance: &Tolerance, size: f64) -> f64 {
    tolerance.angular.max(BEND).max(tolerance.linear / size)
}

fn refuse(edge: EdgeId, face: Option<FaceId>, reason: Reason) -> BlendError {
    BlendError::Refused {
        edge: Some(edge),
        face,
        reason,
    }
}

/// A face's surface and which way is out.
#[derive(Clone, Copy)]
struct Side<'a> {
    surface: &'a Surface,
    reversed: bool,
}

impl<'a> Side<'a> {
    /// The face `face` of `body` as a side: its surface, out of the body.
    fn of(body: &'a Body, face: FaceId) -> Result<Side<'a>, BlendError> {
        let f = body.face(face).ok_or_else(lost)?;
        Ok(Side {
            surface: &f.surface,
            reversed: f.reversed,
        })
    }

    fn normal(&self, (u, v): (f64, f64)) -> Option<Vec3> {
        let n = self.surface.normal(u, v)?;
        Some(if self.reversed { -n } else { n })
    }

    /// The point of the surface nearest `p`, from parameters near it: in
    /// closed form for the analytic surfaces, by Gauss-Newton steps for a
    /// spline surface (whose end spans continue past its domain as the
    /// polynomials they are, so a blend can be continued past the edge of
    /// a lofted face).
    fn foot(&self, p: Vec3, start: (f64, f64)) -> Option<((f64, f64), Vec3)> {
        if !matches!(self.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
            let uv = self.surface.project(p).ok()?;
            return Some((uv, self.surface.eval(uv.0, uv.1)));
        }
        let (mut u, mut v) = start;
        for _ in 0..FOOT_STEPS {
            let (s, su, sv) = self.surface.derivatives(u, v);
            let r = p - s;
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let det = a * c - b * b;
            if !(det.abs() > 0.0) || !det.is_finite() {
                return None;
            }
            let (ru, rv) = (su.dot(r), sv.dot(r));
            let du = (c * ru - b * rv) / det;
            let dv = (a * rv - b * ru) / det;
            u += du;
            v += dv;
            if !(u.is_finite() && v.is_finite()) {
                return None;
            }
            let moved = (su * du + sv * dv).length();
            if moved <= 1e-15 * (1.0 + s.length()) {
                break;
            }
        }
        let point = self.surface.eval(u, v);
        point.is_finite().then_some(((u, v), point))
    }

    /// Signed distance from the surface: positive on the outer side.
    fn signed(&self, p: Vec3, start: (f64, f64)) -> Option<(f64, (f64, f64))> {
        let (uv, foot) = self.foot(p, start)?;
        let n = self.normal(uv)?;
        Some(((p - foot).dot(n), uv))
    }
}

/// Where a plane across the edge is: the point and the unit normal.
#[derive(Clone, Copy, Debug)]
struct Across {
    point: Vec3,
    normal: Vec3,
}

/// The edge as a family of planes across it: `s` from 0 at its start to
/// 1 at its end, and along the curve continued past either end. `range`
/// runs from the curve's parameter at `s = 0` to the one at `s = 1`, so an
/// edge followed against its curve has it the other way round.
struct Spine<'a> {
    curve: &'a Curve3,
    range: (f64, f64),
    /// A closed edge: `s` goes round, a whole turn per unit.
    closed: bool,
    /// Planes the family turns into at `s = 0` and `s = 1` (the joints of
    /// a chain, where the next edge's blend face begins): over `turn` of
    /// `s` from that end the plane across the edge is blended into it, so
    /// the section at the end lies in exactly that plane.
    joints: [Option<Across>; 2],
    turn: f64,
}

impl Spine<'_> {
    /// The plane across the curve at `s`, as it is.
    fn plain(&self, s: f64) -> Option<Across> {
        let (t0, t1) = self.range;
        // Past the ends the curve goes on as it is (a spline's end spans
        // as the polynomials they are), so the planes turn smoothly.
        let s = if self.closed { s.rem_euclid(1.0) } else { s };
        let t = t0 + s * (t1 - t0);
        let (point, d) = self.curve.derivatives(t);
        let normal = (d * (t1 - t0)).normalized()?;
        point.is_finite().then_some(Across { point, normal })
    }

    fn at(&self, s: f64) -> Option<Across> {
        let plain = self.plain(s)?;
        // Near an end with a joint, the plane is moved by what turns the
        // plane across the curve at that end into the joint's, weighted
        // from 1 at the end to 0 at `turn` from it (flat to the third
        // derivative at both): at the end it is the joint's plane, and
        // the planes keep moving along the curve as they do elsewhere.
        let (mut n, mut d) = (plain.normal, plain.normal.dot(plain.point));
        let [first, last] = self.joints;
        for (end, joint) in [(0.0, first), (1.0, last)] {
            let Some(joint) = joint else { continue };
            if !(self.turn > 0.0) {
                return None;
            }
            let x = (1.0 - (s - end).abs() / self.turn).clamp(0.0, 1.0);
            if !(x > 0.0) {
                continue;
            }
            let w = x.powi(4) * (35.0 + x * (-84.0 + x * (70.0 - 20.0 * x)));
            let there = self.plain(end)?;
            n += (joint.normal - there.normal) * w;
            d += (joint.normal.dot(joint.point) - there.normal.dot(there.point)) * w;
        }
        let length = n.length();
        if !(length > 1e-3) {
            return None;
        }
        let normal = n * (1.0 / length);
        let point = plain.point + n * ((d - n.dot(plain.point)) / (length * length));
        (point.is_finite() && normal.is_finite()).then_some(Across { point, normal })
    }
}

/// One section of the blend face: its two ends on the faces (with the
/// faces' parameters there) and, for a fillet, the ball's centre.
#[derive(Clone, Copy, Debug)]
struct Section {
    feet: [Vec3; 2],
    params: [(f64, f64); 2],
    centre: Option<Vec3>,
}

impl Section {
    /// The control points and weights of the section's curve: the
    /// rational quadratic arc of the ball, or the chamfer line.
    fn net(&self) -> Option<Vec<(Vec3, f64)>> {
        let [q1, q2] = self.feet;
        let Some(c) = self.centre else {
            return Some(vec![(q1, 1.0), (q2, 1.0)]);
        };
        let (a, b) = (q1 - c, q2 - c);
        let r = a.length();
        let cosine = (a.dot(b) / (r * b.length())).clamp(-1.0, 1.0);
        let half = 0.5 * cosine.acos();
        // An arc of half a turn or more has no single quadratic.
        if !(half < 0.5 * std::f64::consts::PI * (1.0 - 1e-6)) {
            return None;
        }
        let w = half.cos();
        let middle = (a + b).normalized()?;
        Some(vec![(q1, 1.0), (c + middle * (r / w), w), (q2, 1.0)])
    }

    /// The point at `v` (0 to 1) of the section's curve.
    fn eval(&self, v: f64) -> Option<Vec3> {
        let net = self.net()?;
        let point = match net.as_slice() {
            [(p0, _), (p1, _)] => *p0 * (1.0 - v) + *p1 * v,
            [(p0, w0), (p1, w1), (p2, w2)] => {
                let b = [(1.0 - v) * (1.0 - v), 2.0 * v * (1.0 - v), v * v];
                let w = b[0] * w0 + b[1] * w1 + b[2] * w2;
                (*p0 * (b[0] * w0) + *p1 * (b[1] * w1) + *p2 * (b[2] * w2)) * (1.0 / w)
            }
            _ => return None,
        };
        Some(point)
    }
}

/// What is blended and how.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// A fillet: its radius, and whether the edge is convex.
    Ball { radius: f64, convex: bool },
    /// A chamfer: the distances along the first and the second face.
    Cut { distances: [f64; 2] },
}

impl Shape {
    fn size(&self) -> f64 {
        match *self {
            Shape::Ball { radius, .. } => radius,
            Shape::Cut { distances } => distances[0].max(distances[1]),
        }
    }
}

struct Solver<'a> {
    spine: Spine<'a>,
    sides: [Side<'a>; 2],
    shape: Shape,
    scale: f64,
    /// Where a motion carries the edge and both faces into themselves:
    /// every section is the first one moved by it ([`exact`]).
    moved: Option<exact::Moved>,
}

impl Solver<'_> {
    /// The section in the plane at `s`, started from `near` (a section
    /// nearby).
    fn section(&self, s: f64, near: &Section) -> Option<Section> {
        if let Some(moved) = &self.moved {
            return moved.at(s, self.sides);
        }
        crate::work::blend_section();
        let across = self.spine.at(s)?;
        match self.shape {
            Shape::Ball { radius, convex } => self.ball(across, near, radius, convex),
            Shape::Cut { distances } => self.cut(across, near, distances),
        }
    }

    /// The section at `to`, followed from `near` (the section at `from`):
    /// directly, or where that does not settle (the ball moves a long way
    /// for the step, under a face that curves tightly) through sections
    /// in between, halving the step up to `depth` times.
    fn follow(&self, from: f64, near: &Section, to: f64, depth: u32) -> Option<Section> {
        if let Some(found) = self.section(to, near) {
            return Some(found);
        }
        if depth == 0 {
            return None;
        }
        let middle = 0.5 * (from + to);
        let half = self.follow(from, near, middle, depth - 1)?;
        self.follow(middle, &half, to, depth - 1)
    }

    fn ball(&self, across: Across, near: &Section, radius: f64, convex: bool) -> Option<Section> {
        let inward = if convex { -radius } else { radius };
        let mut centre = near.centre?;
        // Into the plane first.
        centre = centre - across.normal * (centre - across.point).dot(across.normal);
        let mut params = near.params;
        let mut feet = near.feet;
        let mut settled = false;
        for _ in 0..BALL_STEPS {
            let mut normals = [Vec3::ZERO; 2];
            for k in 0..2 {
                let (side, start) = (self.sides.get(k)?, *params.get(k)?);
                let (uv, foot) = side.foot(centre, start)?;
                *params.get_mut(k)? = uv;
                *feet.get_mut(k)? = foot;
                *normals.get_mut(k)? = side.normal(uv)?;
            }
            // n_k · x = n_k · foot_k + inward; normal · x = normal · point.
            let [n1, n2] = normals;
            let [f1, f2] = feet;
            let rows = [n1, n2, across.normal];
            let rhs = [
                n1.dot(f1) + inward,
                n2.dot(f2) + inward,
                across.normal.dot(across.point),
            ];
            let next = solve3(rows, rhs)?;
            // A Newton step on the two distances; never more than half
            // the radius at once, so a poor start (two faces that curve
            // tightly under the ball) does not throw it away.
            let step = next - centre;
            let length = step.length();
            let next = if length > 0.5 * radius {
                centre + step * (0.5 * radius / length)
            } else {
                next
            };
            let moved = next.distance(centre);
            centre = next;
            if moved <= 1e-14 * self.scale {
                settled = true;
                break;
            }
        }
        if !settled {
            return None;
        }
        // The feet of the settled centre, and the ball touches both.
        for k in 0..2 {
            let (side, start) = (self.sides.get(k)?, *params.get(k)?);
            let (uv, foot) = side.foot(centre, start)?;
            *params.get_mut(k)? = uv;
            *feet.get_mut(k)? = foot;
            let n = side.normal(uv)?;
            let offset = centre - foot;
            let off = (offset.length() - radius).abs();
            let square = offset.cross(n).length();
            if !(off <= 1e-9 * self.scale && square <= 1e-7 * radius) {
                return None;
            }
            if offset.dot(n) * inward <= 0.0 {
                return None;
            }
        }
        Some(Section {
            feet,
            params,
            centre: Some(centre),
        })
    }

    fn cut(&self, across: Across, near: &Section, distances: [f64; 2]) -> Option<Section> {
        let mut feet = near.feet;
        let mut params = near.params;
        for k in 0..2 {
            let side = self.sides.get(k)?;
            let d = *distances.get(k)?;
            // Start where the nearby section's foot is, moved into this
            // plane, and walk on the surface: in the plane, `d` from its
            // point.
            let mut uv = *params.get(k)?;
            let mut done = false;
            for _ in 0..FOOT_STEPS {
                let (s, su, sv) = side.surface.derivatives(uv.0, uv.1);
                let off = s - across.point;
                let f = [off.dot(across.normal), off.dot(off) - d * d];
                let j = [
                    [su.dot(across.normal), sv.dot(across.normal)],
                    [2.0 * off.dot(su), 2.0 * off.dot(sv)],
                ];
                let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
                if !(det.abs() > 0.0) || !det.is_finite() {
                    return None;
                }
                let du = (f[0] * j[1][1] - f[1] * j[0][1]) / det;
                let dv = (f[1] * j[0][0] - f[0] * j[1][0]) / det;
                uv = (uv.0 - du, uv.1 - dv);
                if !(uv.0.is_finite() && uv.1.is_finite()) {
                    return None;
                }
                if (su * du + sv * dv).length() <= 1e-15 * self.scale {
                    done = true;
                    break;
                }
            }
            let point = side.surface.eval(uv.0, uv.1);
            let off = point - across.point;
            if !done
                || off.dot(across.normal).abs() > 1e-9 * self.scale
                || (off.length() - d).abs() > 1e-9 * self.scale
            {
                return None;
            }
            *feet.get_mut(k)? = point;
            *params.get_mut(k)? = uv;
        }
        Some(Section {
            feet,
            params,
            centre: None,
        })
    }
}

/// The solution of the three linear equations `rows[i] · x = rhs[i]`.
fn solve3(rows: [Vec3; 3], rhs: [f64; 3]) -> Option<Vec3> {
    let [a, b, c] = rows;
    let det = a.dot(b.cross(c));
    if !(det.abs() > 1e-12) || !det.is_finite() {
        return None;
    }
    let x = (b.cross(c) * rhs[0] + c.cross(a) * rhs[1] + a.cross(b) * rhs[2]) * (1.0 / det);
    x.is_finite().then_some(x)
}

/// The blend face as a surface, from sections at increasing `s`: the
/// surface's `u` is `s`, its `v` runs from the first face to the second.
fn surface_through(params: &[f64], sections: &[Section]) -> Option<NurbsSurface> {
    let nets: Vec<Vec<(Vec3, f64)>> = sections
        .iter()
        .map(Section::net)
        .collect::<Option<Vec<_>>>()?;
    let across = nets.first()?.len();
    if nets.iter().any(|n| n.len() != across) {
        return None;
    }
    let degree = (params.len() - 1).min(3);
    // Per sample, its net's weighted points and weights, in one row.
    let values: Vec<Vec<f64>> = nets
        .iter()
        .map(|net| {
            net.iter()
                .flat_map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
                .collect()
        })
        .collect();
    let (knots_u, rows) = interpolate(params, degree, &values).ok()?;
    let count_u = params.len();
    let mut ctrl = Vec::with_capacity(count_u * across);
    let mut weights = Vec::with_capacity(count_u * across);
    for row in &rows {
        for j in 0..across {
            let at = |k: usize| row.get(4 * j + k).copied();
            let (x, y, z, weight) = (at(0)?, at(1)?, at(2)?, at(3)?);
            if !(weight > 0.0) {
                return None;
            }
            ctrl.push(Vec3::new(x, y, z) * (1.0 / weight));
            weights.push(weight);
        }
    }
    let degree_v = across - 1;
    let mut knots_v = vec![0.0; across];
    knots_v.extend(std::iter::repeat_n(1.0, across));
    NurbsSurface::new(
        degree, degree_v, knots_u, knots_v, count_u, across, ctrl, weights,
    )
    .ok()
}

/// The iso-curve of `surface` at `v = 0` (`last` false) or `v = 1`.
fn border(surface: &NurbsSurface, last: bool) -> Option<NurbsCurve> {
    let (count_u, count_v) = surface.counts();
    let j = if last { count_v.checked_sub(1)? } else { 0 };
    let mut ctrl = Vec::with_capacity(count_u);
    let mut weights = Vec::with_capacity(count_u);
    for i in 0..count_u {
        let index = i.checked_mul(count_v)?.checked_add(j)?;
        ctrl.push(*surface.control_points().get(index)?);
        weights.push(*surface.weights().get(index)?);
    }
    NurbsCurve::new(
        surface.degrees().0,
        surface.knots_u().to_vec(),
        ctrl,
        weights,
    )
    .ok()
}

/// A polynomial cubic (or lower) through `points` at `params`.
fn curve_through(params: &[f64], points: &[Vec3]) -> Option<NurbsCurve> {
    let degree = (params.len().checked_sub(1)?).min(3);
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, rows) = interpolate(params, degree, &values).ok()?;
    let ctrl: Vec<Vec3> = rows
        .iter()
        .map(|r| match r.as_slice() {
            [x, y, z] => Some(Vec3::new(*x, *y, *z)),
            _ => None,
        })
        .collect::<Option<_>>()?;
    NurbsCurve::polynomial(degree, knots, ctrl).ok()
}

/// The vertex at one end of the edge and what meets there.
#[derive(Clone, Copy, Debug)]
struct End {
    vertex: VertexId,
    /// The other edge of the first face and of the second face there.
    others: [EdgeId; 2],
    /// The third face.
    third: FaceId,
    /// The edge runs on into the material behind the third face (it ends
    /// in a wall), so the blend is cut where it goes into it.
    facing: bool,
    /// The sine of the angle at which the edge runs past its end into or
    /// out of the third face (at least the bound the blend's normals are
    /// known to): a blend has to go on past the end the longer the more
    /// nearly it runs along the face.
    slant: f64,
    /// Per side, a smooth edge of that side's face which the boundary
    /// with the third face may be followed across, past this end, into
    /// the face beyond it: where the blend's border crosses that edge
    /// (see [`cross`]).
    across: [Option<EdgeId>; 2],
}

/// Per face, its edges; per edge, its faces.
fn faces_of_edge(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    let mut found = Vec::new();
    for (index, face) in body.faces.iter().enumerate() {
        let uses = face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
            .any(|c| c.edge == edge);
        if uses {
            found.push(FaceId(u32::try_from(index).unwrap_or(u32::MAX)));
        }
    }
    found
}

/// `bound` is the sine to which the blend's normals are known (what its
/// faces are refined to): an edge that runs past its end closer than that
/// to along the third face is taken as running out of it, and the end cut
/// decides.
fn end_at(
    body: &Body,
    edge: EdgeId,
    faces: [FaceId; 2],
    vertex: VertexId,
    bound: f64,
) -> Result<End, BlendError> {
    let at: Vec<EdgeId> = body
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.start == vertex || e.end == vertex)
        .map(|(i, _)| EdgeId(u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let others: Vec<EdgeId> = at.iter().copied().filter(|e| *e != edge).collect();
    let [x, y] = others.as_slice() else {
        return Err(refuse(edge, None, Reason::FreeEnd));
    };
    let (fx, fy) = (faces_of_edge(body, *x), faces_of_edge(body, *y));
    let [first, second] = faces;
    let pick = |fs: &[FaceId], own: FaceId| -> Option<FaceId> {
        match fs {
            [a, b] if *a == own => Some(*b),
            [a, b] if *b == own => Some(*a),
            _ => None,
        }
    };
    let (ox, oy) = if fx.contains(&first) && fy.contains(&second) {
        (*x, *y)
    } else if fy.contains(&first) && fx.contains(&second) {
        (*y, *x)
    } else {
        return Err(refuse(edge, None, Reason::FreeEnd));
    };
    // An edge that goes on straight into another edge here is the middle
    // of a chain, not an end the blend can be cut at.
    if goes_on(body, edge, vertex, [ox, oy]) {
        return Err(refuse(edge, None, Reason::FreeGoesOn));
    }
    let (fa, fb) = (faces_of_edge(body, ox), faces_of_edge(body, oy));
    let (Some(t1), Some(t2)) = (pick(&fa, first), pick(&fb, second)) else {
        return Err(refuse(edge, None, Reason::FreeEnd));
    };
    if t1 != t2 || t1 == first || t1 == second {
        return Err(refuse(edge, None, Reason::FreeEnd));
    }
    let start_or_end = |e: EdgeId| body.edge(e).is_some_and(|d| d.start != d.end);
    if !(start_or_end(ox) && start_or_end(oy)) {
        return Err(refuse(edge, None, Reason::FreeEnd));
    }
    let (facing, slant) = runs_into(body, edge, vertex, t1, bound)
        .ok_or_else(|| refuse(edge, Some(t1), Reason::FreeEnd))?;
    Ok(End {
        vertex,
        others: [ox, oy],
        third: t1,
        facing,
        slant,
        across: [None, None],
    })
}

/// Whether `edge`, arriving at `vertex`, leaves it again along one of
/// `others`: by the test `tangent_chain` picks the edges of a chain with
/// (their directions' cosine within 1e-9 of one), so the chain is what
/// the refusal asks for.
fn goes_on(body: &Body, edge: EdgeId, vertex: VertexId, others: [EdgeId; 2]) -> bool {
    let leaving = |e: EdgeId| -> Option<Vec3> {
        let d = body.edge(e)?;
        let (t, sign) = if d.start == vertex {
            (d.range.0, 1.0)
        } else {
            (d.range.1, -1.0)
        };
        let (_, der) = d.curve.derivatives(t);
        (der * sign).normalized()
    };
    let Some(back) = leaving(edge) else {
        return false;
    };
    others
        .into_iter()
        .any(|o| leaving(o).is_some_and(|l| -back.dot(l) > 1.0 - 1e-9))
}

/// Whether the edge, continued past its end at `vertex`, runs into the
/// material behind the face `third` (a wall the edge ends in, at
/// concave edges) rather than out of it (an end face, at convex ones),
/// told only where the edge's direction is off the face by more than
/// `bound`; with the sine of that angle (at least `bound`). `None` where
/// the direction or the face's normal there cannot be had.
fn runs_into(
    body: &Body,
    edge: EdgeId,
    vertex: VertexId,
    third: FaceId,
    bound: f64,
) -> Option<(bool, f64)> {
    let data = body.edge(edge)?;
    let (t, sign) = if data.start == vertex {
        (data.range.0, -1.0)
    } else {
        (data.range.1, 1.0)
    };
    let (point, d) = data.curve.derivatives(t);
    let out = (d * sign).normalized()?;
    let side = Side::of(body, third).ok()?;
    let uv = side.surface.project(point).ok()?;
    let n = side.normal(uv)?;
    // Into the material behind the face only when that is told beyond what
    // the normals are known to; otherwise out of it, as every end was
    // taken before, and the end cut decides whether it is clean.
    let along = n.dot(out);
    Some((along < -bound, along.abs().max(bound)))
}

impl End {
    /// How far past this end a blend may be followed before it is given up
    /// as not leaving the third face: the steps' allowance for an end face
    /// square to the edge, over the sine at which the edge meets it.
    fn steps(&self, steps: Steps) -> Steps {
        Steps {
            most_out: steps.most_out / self.slant,
            ..steps
        }
    }

    /// The third face as the side a blend has to get out of past this
    /// end: turned round where the blend runs into the material behind
    /// it.
    fn side<'a>(&self, body: &'a Body) -> Result<Side<'a>, BlendError> {
        let face = Side::of(body, self.third)?;
        Ok(Side {
            reversed: face.reversed != self.facing,
            ..face
        })
    }
}

/// The blend face of one edge, worked out: its surface, the samples it
/// went through, and where it is cut at each end.
struct Made {
    surface: NurbsSurface,
    /// Per end: the cut curve (from the first face to the second) and the
    /// parameter `s` at which each border meets it.
    cuts: [(NurbsCurve, [f64; 2]); 2],
}

/// Fills `params`/`sections` until the surface through them is within
/// `limit` of the exact sections everywhere between two samples.
fn refine(
    solver: &Solver<'_>,
    params: &mut Vec<f64>,
    sections: &mut Vec<Section>,
    limit: f64,
    bend: f64,
) -> Option<NurbsSurface> {
    refine_samples(solver, params, sections, limit, bend, false)
}

/// [`refine`] for an open surface (through the samples, `surface_through`)
/// or a closed one (`closed`: round them, [`ring_surface`], the last
/// sample followed by the first one turn on). Each pass holds the surface
/// at the middle of every pair of samples against the exact section there
/// (to `limit`, and a fillet's normal along its borders and across it to
/// `bend`) and adds the middles where it does not hold, and next to them
/// those that adding them would push over (see [`to_split`]). Once the
/// middles hold, each border is proven within `limit` of its face between
/// every two samples (`geom::certify`, at most [`STEP_PROOF_PIECES`]
/// pieces a step), and a step where it is not is halved. At most
/// [`PASSES`] passes and [`MAX_SAMPLES`] samples.
fn refine_samples(
    solver: &Solver<'_>,
    params: &mut Vec<f64>,
    sections: &mut Vec<Section>,
    limit: f64,
    bend: f64,
    closed: bool,
) -> Option<NurbsSurface> {
    let faces = [
        SurfaceBound::new(solver.sides[0].surface)?,
        SurfaceBound::new(solver.sides[1].surface)?,
    ];
    for _ in 0..PASSES {
        let surface = if closed {
            ring_surface(params, sections)?
        } else {
            surface_through(params, sections)?
        };
        let first = *params.first()?;
        let pairs = if closed {
            params.len()
        } else {
            params.len().saturating_sub(1)
        };
        // Per pair of samples: how far the surface is over its bounds at
        // the middle, the middle and its section, and whether to add it.
        let mut middles: Vec<(f64, f64, Section, bool)> = Vec::with_capacity(pairs);
        for index in 0..pairs {
            let a = *params.get(index)?;
            let b = match params.get(index + 1) {
                Some(b) => *b,
                None if closed => first + 1.0,
                None => return None,
            };
            let near = sections.get(index)?;
            let s = 0.5 * (a + b);
            let exact = solver.section(s, near)?;
            let mut worst = 0.0_f64;
            for k in 0..=8 {
                let v = f64::from(k) / 8.0;
                let d = surface.eval(s, v).distance(exact.eval(v)?);
                worst = worst.max(d);
            }
            // Tangent to the faces along its borders, to `bend`.
            let mut kink = 0.0_f64;
            if matches!(solver.shape, Shape::Ball { .. }) {
                for (k, v) in [(0_usize, 0.0), (1, 1.0)] {
                    let (point, du, dv) = surface.derivatives(s, v);
                    let side = solver.sides.get(k)?;
                    let start = *exact.params.get(k)?;
                    let (uv, _) = side.foot(point, start)?;
                    let n = side.normal(uv)?;
                    let own = du.cross(dv).normalized()?;
                    kink = kink.max(own.cross(n).length());
                }
                // Across the face, along the ball's radius: here, and at
                // the first and the last sample of an open face (where
                // the next face of a chain goes on from it, and has to be
                // tangent).
                kink = kink.max(radial_kink(&surface, s, &exact)?);
                if !closed && index == 0 {
                    kink = kink.max(radial_kink(&surface, a, near)?);
                }
                if !closed && index + 1 == pairs {
                    let last = sections.get(index + 1)?;
                    kink = kink.max(radial_kink(&surface, b, last)?);
                }
            }
            // How far over its bound the worse of the two is (NaN: over).
            let over = (worst / limit).max(kink / bend);
            middles.push((
                if over.is_nan() { f64::INFINITY } else { over },
                s,
                exact,
                false,
            ));
        }
        if middles.iter().all(|m| m.0 <= 1.0) {
            // Held at the middles: between the samples each border is
            // proven on its face (`geom::certify`), and a step where it is
            // not is halved.
            let mut unproven = false;
            for (k, face) in faces.iter().enumerate() {
                let curve = border(&surface, k == 1)?;
                let curve = CurveBound::nurbs(&curve);
                for (index, m) in middles.iter_mut().enumerate() {
                    let a = *params.get(index)?;
                    let b = params.get(index + 1).copied().unwrap_or(first + 1.0);
                    let start = *sections.get(index)?.params.get(k)?;
                    if face
                        .curve_within(&curve, (a, b), start, &[], limit, STEP_PROOF_PIECES)
                        .is_err()
                    {
                        m.0 = f64::INFINITY;
                        unproven = true;
                    }
                }
            }
            if !unproven {
                return Some(surface);
            }
        }
        let over: Vec<f64> = middles.iter().map(|m| m.0).collect();
        for (m, split) in middles.iter_mut().zip(to_split(&over, closed)) {
            m.3 = split;
        }
        let added: Vec<(f64, Section)> = middles
            .into_iter()
            .filter(|m| m.3)
            .map(|(_, s, exact, _)| (if closed { s.rem_euclid(1.0) } else { s }, exact))
            .collect();
        if params.len() + added.len() > MAX_SAMPLES {
            return None;
        }
        for (s, section) in added {
            let at = params.partition_point(|p| *p < s);
            params.insert(at, s);
            sections.insert(at, section);
        }
    }
    None
}

/// Which steps of an interpolating cubic to halve, from how far over its
/// bound the cubic is at the middle of each (`over`, 1 at the bound;
/// round, when `closed`): every step over it, and every step that the
/// halving of one over it can push over it. Halving a step makes the
/// cubic pass through the exact middle, which moves it there by what it
/// was off; through the spline's equations with continuous curvature a
/// change at one sample dies away by the factor 2 − √3 per sample (the
/// root of t[i-1] + 4 t[i] + t[i+1] = 0, the recurrence of evenly spaced
/// samples), so a step k samples away moves by about (2 − √3)^k times as
/// much and is halved now when that would take it over its bound. Halving
/// only what is over the bound makes the cubic creep: each pass pushes
/// the steps next to the halved ones over it, one more at a time, until
/// the passes run out (seen on the rings of welds between tubes).
fn to_split(over: &[f64], closed: bool) -> Vec<bool> {
    let decay = 2.0 - 3.0_f64.sqrt();
    let n = over.len();
    let mut split: Vec<bool> = over.iter().map(|o| !(*o <= 1.0)).collect();
    let largest_within = over
        .iter()
        .copied()
        .filter(|o| *o <= 1.0)
        .fold(0.0_f64, f64::max);
    for (i, &o) in over.iter().enumerate() {
        if o <= 1.0 {
            continue;
        }
        // Not a number (nothing to measure there): halved itself only.
        if !o.is_finite() {
            continue;
        }
        let mut push = o;
        for k in 1..n {
            push *= decay;
            if push + largest_within <= 1.0 {
                break;
            }
            let near = [
                i.checked_sub(k).or_else(|| closed.then(|| (i + n - k) % n)),
                (i + k < n)
                    .then_some(i + k)
                    .or_else(|| closed.then(|| (i + k) % n)),
            ];
            for j in near.into_iter().flatten() {
                if over.get(j).is_some_and(|x| x + push > 1.0)
                    && let Some(slot) = split.get_mut(j)
                {
                    *slot = true;
                }
            }
        }
    }
    split
}

/// How far the normal of a fillet's surface at `s` turns away from the
/// radius of the ball `section` there, the largest sine across the face.
fn radial_kink(surface: &NurbsSurface, s: f64, section: &Section) -> Option<f64> {
    let centre = section.centre?;
    let mut kink = 0.0_f64;
    for k in 1..4 {
        let v = f64::from(k) / 4.0;
        let (_, du, dv) = surface.derivatives(s, v);
        let own = du.cross(dv).normalized()?;
        let radius = (section.eval(v)? - centre).normalized()?;
        kink = kink.max(own.cross(radius).length());
    }
    Some(kink)
}

/// Where the blend surface crosses the third face's surface at one end:
/// per `v`, the `s` of the crossing, found between `inner` (inside the
/// third face's surface) and `outer`.
fn crossing(
    surface: &NurbsSurface,
    third: Side<'_>,
    v: f64,
    inner: f64,
    outer: f64,
    scale: f64,
) -> Option<f64> {
    // `inner` is a few radii before the end: the nearest place behind
    // the third face's surface from there inwards, halfway at most (from
    // further in, a curved third face may be nearer on its other side).
    let middle = 0.5;
    let mut from = inner;
    let found = loop {
        let start = third.surface.project(surface.eval(from, v)).ok()?;
        let g = |s: f64| third.signed(surface.eval(s, v), start).map(|x| x.0);
        if g(from)? < 0.0 {
            break (start, from);
        }
        if (from - middle).abs() <= 1e-12 {
            return None;
        }
        // Halfway to the middle each time.
        let next = from + (middle - from) * 0.5;
        from = if (middle - next).abs() < 1e-3 {
            middle
        } else {
            next
        };
    };
    let (start, inner) = found;
    let g = |s: f64| third.signed(surface.eval(s, v), start).map(|x| x.0);
    let (mut lo, mut hi) = (inner, outer);
    let (mut glo, ghi) = (g(lo)?, g(hi)?);
    if !(glo < 0.0 && ghi > 0.0) {
        return None;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let gm = g(mid)?;
        if gm < 0.0 {
            lo = mid;
            glo = gm;
        } else {
            hi = mid;
        }
        if glo.abs() <= 1e-15 * scale {
            // On the surface already: that is the crossing, not the
            // middle of what is left of the bracket.
            return Some(lo);
        }
        if (hi - lo).abs() <= 1e-15 * (1.0 + mid.abs()) {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// The curve in which the blend surface crosses the third face at one
/// end, fitted within `limit` to points found exactly; and the `s` of its
/// two ends (on the first face's border, on the second's).
fn cut_curve(
    surface: &NurbsSurface,
    third: Side<'_>,
    inner: f64,
    outer: f64,
    limit: f64,
    scale: f64,
) -> Option<(NurbsCurve, [f64; 2])> {
    let mut vs: Vec<f64> = (0..=8).map(|k| f64::from(k) / 8.0).collect();
    for _ in 0..12 {
        let ss: Vec<f64> = vs
            .iter()
            .map(|v| crossing(surface, third, *v, inner, outer, scale))
            .collect::<Option<Vec<_>>>()?;
        let points: Vec<Vec3> = vs
            .iter()
            .zip(&ss)
            .map(|(v, s)| surface.eval(*s, *v))
            .collect();
        let curve = curve_through(&vs, &points)?;
        let mut added = Vec::new();
        for pair in vs.windows(2) {
            let [a, b] = pair else { continue };
            let v = 0.5 * (a + b);
            let s = crossing(surface, third, v, inner, outer, scale)?;
            if !(curve.eval(v).distance(surface.eval(s, v)) <= limit) {
                added.push(v);
            }
        }
        // Between the middles: proven on both surfaces, the blend's and
        // the third face's; where not, the step the proof stopped in is
        // halved too.
        if added.is_empty() {
            let (Some(&s0), Some(&v0), Some(&p0)) = (ss.first(), vs.first(), points.first()) else {
                return None;
            };
            let on_third = third.surface.project(p0).ok()?;
            for (on, start) in [
                (&Surface::Nurbs(surface.clone()), (s0, v0)),
                (third.surface, on_third),
            ] {
                if let Err(at) = proven_on(&curve, (0.0, 1.0), on, start, &vs, limit) {
                    let k = vs.partition_point(|v| *v <= at);
                    if let (Some(a), Some(b)) =
                        (k.checked_sub(1).and_then(|i| vs.get(i)), vs.get(k))
                    {
                        added.push(0.5 * (a + b));
                    } else {
                        return None;
                    }
                    break;
                }
            }
        }
        if added.is_empty() {
            let (Some(&first), Some(&last)) = (ss.first(), ss.last()) else {
                return None;
            };
            return Some((curve, [first, last]));
        }
        if vs.len() + added.len() > MAX_SAMPLES {
            return None;
        }
        vs.extend(added);
        vs.sort_by(f64::total_cmp);
    }
    None
}

/// Most pieces a proof that a fitted curve lies on a surface may take
/// (`geom::certify`): bounded work; a proof that needs more is not had,
/// and the curve is refined or refused.
const PROOF_PIECES: usize = 1 << 16;
/// Most pieces the proof that a blend face's border stays on its face may
/// take over one step between two samples: a step that needs more is
/// halved instead.
const STEP_PROOF_PIECES: usize = 1024;

/// Whether every point of `curve` over `range` is proven within `limit`
/// of `surface` (`start`: the surface's parameters near the curve's
/// point at `range.0`; `breaks`: where the curve was fitted through
/// points of the surface). Where it is not, the curve's parameter where
/// the proof stopped.
fn proven_on(
    curve: &NurbsCurve,
    range: (f64, f64),
    surface: &Surface,
    start: (f64, f64),
    breaks: &[f64],
    limit: f64,
) -> Result<(), f64> {
    let middle = 0.5 * (range.0 + range.1);
    let Some(bound) = SurfaceBound::new(surface) else {
        return Err(middle);
    };
    bound
        .curve_within(
            &CurveBound::nurbs(curve),
            range,
            start,
            breaks,
            limit,
            PROOF_PIECES,
        )
        .map(|_| ())
        .map_err(|stop| stop.at)
}

/// Blends `request` of `body` (an edge none of the closed forms covers)
/// in place: see the module documentation.
fn blend_one(
    body: &mut Body,
    request: &Request,
    [first, second]: [FaceId; 2],
    into: [Vec3; 2],
    convex: bool,
    tolerance: &Tolerance,
) -> Result<FaceId, BlendError> {
    let edge = request.edge;
    let tol = tolerance.linear;
    let data = body.edge(edge).ok_or_else(lost)?.clone();
    let closed = data.start == data.end;
    if closed
        && body
            .edges
            .iter()
            .enumerate()
            .any(|(i, e)| i != edge.0 as usize && (e.start == data.start || e.end == data.start))
    {
        // A ring whose vertex other edges share (a seam through it).
        return Err(refuse(edge, None, Reason::FreeClosed));
    }
    let shape = match request.size {
        Size::Radius(radius) => Shape::Ball { radius, convex },
        Size::Chamfer(ChamferSize::Distances { first, second }) => Shape::Cut {
            distances: [first, second],
        },
        Size::Chamfer(ChamferSize::DistanceAngle { .. }) => {
            return Err(refuse(edge, None, Reason::FreeAngle));
        }
    };
    let sides = [Side::of(body, first)?, Side::of(body, second)?];
    let scale = scale_of(&data, shape.size(), tol);
    let solver = Solver {
        spine: Spine {
            curve: &data.curve,
            range: data.range,
            closed,
            joints: [None, None],
            turn: 0.0,
        },
        sides,
        shape,
        scale,
        moved: None,
    };
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    let seed = first_section(&solver, 0.5, into, edge)?;
    if closed {
        let (surface, params, sections) = ring(&solver, seed, &data, tolerance)
            .ok_or_else(|| refuse(edge, None, Reason::FreeApproximation))?;
        let outward =
            outward_from(&solver, &surface, &params, &sections, convex).ok_or_else(unsolvable)?;
        let (blend_face, second_border) =
            splice_ring(body, edge, [first, second], surface, outward)?;
        // A border that runs off its face first (the more telling reason),
        // then a loop left behind.
        for (border, face) in [(edge, first), (second_border, second)] {
            stays_on(body, edge, border, face, tolerance)?;
        }
        no_loop_behind(body, blend_face, convex, &data.curve, edge, tolerance)?;
        return tangent_or_refused(
            body,
            edge,
            [first, second],
            blend_face,
            second_border,
            shape,
            tolerance,
        );
    }
    let end_bound = bend(tolerance, shape.size());
    let mut ends = [
        end_at(body, edge, [first, second], data.start, end_bound)?,
        end_at(body, edge, [first, second], data.end, end_bound)?,
    ];
    // Smooth edges of the faces beside the edge that the borders cross.
    let crossings = cross::find(
        body,
        &cross::Blend {
            edge,
            data: &data,
            faces: [first, second],
            sides,
            shape,
            ends: &ends,
            into,
            tol,
        },
        tolerance,
    )?;
    for c in &crossings {
        for (j, end) in ends.iter_mut().enumerate() {
            if (c.at.is_none() || c.end == j)
                && let Some(slot) = end.across.get_mut(c.side)
            {
                *slot = Some(c.smooth);
            }
        }
    }
    let built = blend_open(
        body,
        edge,
        [first, second],
        into,
        convex,
        shape,
        scale,
        &data,
        ends,
        &crossings,
        seed,
        tolerance,
    );
    if crossings.is_empty() {
        return built;
    }
    built.map_err(|error| match error {
        // A border that cannot be carried across (no ball rolls against
        // the faces beyond, or none within the tolerance) runs off its
        // face, as it did before it was carried: which `fit` reads as no
        // room.
        BlendError::Refused {
            reason: Reason::FreeSurface | Reason::FreeApproximation | Reason::NotTangent,
            ..
        } => refuse(
            edge,
            crossings.first().map(|c| c.before),
            Reason::FreeRunsOver,
        ),
        other => other,
    })
}

/// The blend face of an open edge, from `seed` (its section at the
/// middle, against the edge's own faces): sections from the middle out
/// past both ends, refined, cut at the ends against the third faces and
/// spliced in. Where a border crosses smooth edges of its face
/// ([`cross`]) the sections on each side of a switch are found against
/// the faces there, the pieces refined each on their own faces and joined
/// across the section at the switch (which both pass through), and after
/// the splice the borders are cut into pieces on the faces they lie on;
/// with no crossing there is one piece, on the edge's faces. Under a
/// motion that carries the edge and both faces of the one piece into
/// themselves the sections are the seed moved ([`exact`]).
#[allow(clippy::too_many_arguments)]
fn blend_open(
    body: &mut Body,
    edge: EdgeId,
    faces: [FaceId; 2],
    into: [Vec3; 2],
    convex: bool,
    shape: Shape,
    scale: f64,
    data: &Edge,
    ends: [End; 2],
    crossings: &[cross::Crossing],
    seed: Section,
    tolerance: &Tolerance,
) -> Result<FaceId, BlendError> {
    let tol = tolerance.linear;
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    let [first, _] = faces;
    let runs_over = || refuse(edge, Some(first), Reason::FreeRunsOver);
    let breaks = cross::switches(crossings);
    if breaks.windows(2).any(|w| matches!(w, [a, b] if !(b > a))) {
        return Err(runs_over());
    }
    // The pieces of the blend between the switches, and the faces each
    // touches.
    let mut bounds = vec![f64::NEG_INFINITY];
    bounds.extend(breaks.iter().copied());
    bounds.push(f64::INFINITY);
    let mut pieces: Vec<[FaceId; 2]> = Vec::with_capacity(bounds.len());
    for w in bounds.windows(2) {
        let [lo, hi] = w else { return Err(lost()) };
        let middle = match (lo.is_finite(), hi.is_finite()) {
            (true, true) => 0.5 * (lo + hi),
            (false, true) => hi - 1.0,
            (true, false) => lo + 1.0,
            (false, false) => 0.5,
        };
        pieces.push([
            cross::face_at(faces, crossings, 0, middle),
            cross::face_at(faces, crossings, 1, middle),
        ]);
    }
    let spine = || Spine {
        curve: &data.curve,
        range: data.range,
        closed: false,
        joints: [None, None],
        turn: 0.0,
    };
    let mut solvers = Vec::with_capacity(pieces.len());
    for [a, b] in &pieces {
        solvers.push(Solver {
            spine: spine(),
            sides: [Side::of(body, *a)?, Side::of(body, *b)?],
            shape,
            scale,
            moved: None,
        });
    }
    let home = bounds
        .windows(2)
        .position(|w| matches!(w, [lo, hi] if *lo <= 0.5 && 0.5 < *hi))
        .ok_or_else(lost)?;
    // The seed holds where the middle of the blend touches the edge's own
    // faces; past a switch at the middle it is found against the faces
    // there.
    let seed = if pieces.get(home) == Some(&faces) {
        seed
    } else {
        first_section(solvers.get(home).ok_or_else(lost)?, 0.5, into, edge)?
    };
    // One piece: the sections may be the seed moved.
    if let [only] = solvers.as_mut_slice() {
        let motion = exact::motion(&only.spine, only.sides);
        only.moved = motion.and_then(|m| exact::Moved::new(m, &only.spine, seed, 0.5));
    }
    let length = edge_length(data);
    if !(length > 0.0) {
        return Err(unsolvable());
    }
    let size = shape.size();
    let steps = Steps::of(size, length);
    let thirds: Vec<Side<'_>> = ends
        .iter()
        .map(|end| end.side(body))
        .collect::<Result<_, _>>()?;
    // Sections from the middle outwards, each from its neighbour, across
    // the switches: the section at a switch, found against the faces
    // before it, is the ball that touches the faces after it too.
    let mut tagged: Vec<(f64, usize, Section)> = vec![(0.5, home, seed)];
    let mut reach = [0.0_f64; 2];
    for (index, end) in [(0_usize, 0.0_f64), (1, 1.0)] {
        let end_data = ends.get(index).ok_or_else(lost)?;
        let third = *thirds.get(index).ok_or_else(lost)?;
        let allowed = end_data.steps(steps);
        let mut piece = home;
        let mut at = (0.5, seed);
        loop {
            let (next, bound) = if index == 1 {
                (piece + 1, bounds.get(piece + 1))
            } else {
                match piece.checked_sub(1) {
                    Some(p) => (p, bounds.get(piece)),
                    None => break,
                }
            };
            let Some(&bound) = bound.filter(|b| b.is_finite()) else {
                break;
            };
            // A switch further past the end than the blend may be followed.
            let past = (bound - end) * (2.0 * end - 1.0);
            if past > allowed.most_out {
                return Err(runs_over());
            }
            let solver = solvers.get(piece).ok_or_else(lost)?;
            let more = march_to(solver, at, bound, steps.inside).ok_or_else(unsolvable)?;
            let (b, last) = *more.last().ok_or_else(lost)?;
            tagged.extend(more.into_iter().map(|(s, x)| (s, piece, x)));
            let beyond = solvers.get(next).ok_or_else(lost)?;
            let moved = cross::moved_onto(&last, beyond.sides, tol).ok_or_else(runs_over)?;
            // The ball holds for the faces beyond the switch too.
            let again = beyond.section(b, &moved).ok_or_else(unsolvable)?;
            let same = match (again.centre, moved.centre) {
                (Some(p), Some(q)) => p.distance(q) <= 0.25 * tol,
                (None, None) => true,
                _ => false,
            };
            let feet = again
                .feet
                .iter()
                .zip(&moved.feet)
                .all(|(p, q)| p.distance(*q) <= 0.25 * tol);
            if !(same && feet) {
                return Err(refuse(edge, None, Reason::NotTangent));
            }
            tagged.push((b, next, moved));
            piece = next;
            at = (b, moved);
        }
        let solver = solvers.get(piece).ok_or_else(lost)?;
        let (more, past) =
            march_out(solver, at, end, third, allowed, tol).map_err(|stop| match stop {
                Stop::Unsolvable => unsolvable(),
                Stop::TooFar => refuse(edge, Some(end_data.third), Reason::FreeEnd),
            })?;
        tagged.extend(more.into_iter().map(|(s, x)| (s, piece, x)));
        if let Some(slot) = reach.get_mut(index) {
            *slot = past;
        }
    }
    let limit = 0.25 * tol;
    let bend = bend(tolerance, size);
    let mut parts = Vec::with_capacity(solvers.len());
    let mut all: Vec<(f64, Section)> = Vec::new();
    for (index, solver) in solvers.iter().enumerate() {
        let mut mine: Vec<(f64, Section)> = tagged
            .iter()
            .filter(|(_, p, _)| *p == index)
            .map(|(s, _, x)| (*s, *x))
            .collect();
        mine.sort_by(|a, b| a.0.total_cmp(&b.0));
        mine.dedup_by(|a, b| a.0 == b.0);
        if mine.len() < 2 {
            return Err(runs_over());
        }
        // Cubic along the blend, for the pieces to join: at least four
        // samples.
        while mine.len() < 4 {
            let widest = mine
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| match w {
                    [a, b] => Some((i, b.0 - a.0)),
                    _ => None,
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let (at, _) = widest.ok_or_else(lost)?;
            let (a, near) = *mine.get(at).ok_or_else(lost)?;
            let b = mine.get(at + 1).ok_or_else(lost)?.0;
            let middle = 0.5 * (a + b);
            let found = solver
                .follow(a, &near, middle, FOLLOW_DEPTH)
                .ok_or_else(unsolvable)?;
            mine.insert(at + 1, (middle, found));
        }
        let mut params: Vec<f64> = mine.iter().map(|x| x.0).collect();
        let mut sections: Vec<Section> = mine.iter().map(|x| x.1).collect();
        let surface = match solver.moved {
            Some(_) => {
                let (Some(p0), Some(p1), Some(c0), Some(c1)) = (
                    params.first(),
                    params.last(),
                    sections.first(),
                    sections.last(),
                ) else {
                    return Err(unsolvable());
                };
                surface_through(&[*p0, *p1], &[*c0, *c1])
            }
            None => refine(solver, &mut params, &mut sections, limit, bend),
        }
        .ok_or_else(|| refuse(edge, None, Reason::FreeApproximation))?;
        parts.push(surface);
        for (s, x) in params.into_iter().zip(sections) {
            if all.last().is_none_or(|l| l.0 < s) {
                all.push((s, x));
            }
        }
    }
    let surface = match parts.as_slice() {
        [only] => only.clone(),
        _ => cross::join(&parts).ok_or_else(|| refuse(edge, None, Reason::FreeApproximation))?,
    };
    let mut cuts = Vec::with_capacity(2);
    for (index, end) in ends.iter().enumerate() {
        let third = *thirds.get(index).ok_or_else(lost)?;
        let past = *reach.get(index).ok_or_else(lost)?;
        let cut = end_cut(
            &surface,
            third,
            index == 1,
            past,
            size / length,
            limit,
            scale,
        )
        .ok_or_else(|| refuse(edge, Some(end.third), Reason::FreeEnd))?;
        cuts.push(cut);
    }
    let [cut0, cut1] = <[(NurbsCurve, [f64; 2]); 2]>::try_from(cuts).map_err(|_| lost())?;
    let params: Vec<f64> = all.iter().map(|x| x.0).collect();
    let sections: Vec<Section> = all.iter().map(|x| x.1).collect();
    let home_solver = solvers.get(home).ok_or_else(lost)?;
    let outward_hint =
        outward_from(home_solver, &surface, &params, &sections, convex).ok_or_else(unsolvable)?;
    let motion = match solvers.as_slice() {
        [only] => only.moved.map(|m| m.motion),
        _ => None,
    };
    drop(solvers);
    drop(thirds);
    let made = Made {
        surface,
        cuts: [cut0, cut1],
    };
    let (blend_face, second_border) =
        splice(body, edge, faces, ends, made, outward_hint, tolerance)?;
    let borders = cross::carry(
        body,
        edge,
        blend_face,
        [edge, second_border],
        faces,
        crossings,
        tol,
    )?;
    for (border, face) in &borders {
        stays_on(body, edge, *border, *face, tolerance)?;
    }
    no_loop_behind(body, blend_face, convex, &data.curve, edge, tolerance)?;
    if let Some(motion) = motion {
        let surface = exact::swept(motion, &seed, shape).ok_or_else(unsolvable)?;
        exact::make_exact(body, blend_face, surface, tolerance)?;
    }
    tangent_along(body, edge, &borders, blend_face, shape, tolerance)?;
    Ok(blend_face)
}

/// Sections from `from` to `to` in even steps of at most `step`, each
/// followed from the one before; the last one at `to`.
fn march_to(
    solver: &Solver<'_>,
    from: (f64, Section),
    to: f64,
    step: f64,
) -> Option<Vec<(f64, Section)>> {
    let (s0, mut last) = from;
    let count = ((to - s0).abs() / step).ceil();
    if !(count.is_finite() && count >= 0.0 && count < MAX_SAMPLES as f64) {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (count as usize).max(1);
    let mut out = Vec::with_capacity(count);
    let mut previous = s0;
    for i in 1..=count {
        let s = if i == count {
            to
        } else {
            s0 + (to - s0) * (i as f64 / count as f64)
        };
        last = solver.follow(previous, &last, s, FOLLOW_DEPTH)?;
        out.push((s, last));
        previous = s;
    }
    Some(out)
}

/// The size of the numbers about an edge: what "settled" is measured
/// against.
fn scale_of(data: &Edge, size: f64, tol: f64) -> f64 {
    let start = data.curve.eval(data.range.0);
    let finish = data.curve.eval(data.range.1);
    start.length().max(finish.length()).max(size).max(tol)
}

/// The length of an edge, roughly: the larger of its chord and the
/// distance from its start to its middle.
fn edge_length(data: &Edge) -> f64 {
    let start = data.curve.eval(data.range.0);
    let finish = data.curve.eval(data.range.1);
    start.distance(finish).max(
        data.curve
            .eval(0.5 * (data.range.0 + data.range.1))
            .distance(start),
    )
}

/// The steps in `s` between sections: one along the edge, and a smaller
/// one past its ends, so the samples are evenly spaced (an interpolating
/// spline overshoots where they are not); and how far past an end a
/// blend may go before it is refused.
#[derive(Clone, Copy, Debug)]
struct Steps {
    inside: f64,
    outside: f64,
    most_out: f64,
}

impl Steps {
    fn of(size: f64, length: f64) -> Steps {
        let inside = (size / length).clamp(1.0 / 200.0, 1.0 / 24.0);
        Steps {
            inside,
            outside: inside.min(0.5 * size / length),
            most_out: 8.0 * size / length,
        }
    }
}

/// The first section of a blend, in the plane at `s`: from the flat
/// estimate (the ball between the two tangent planes along `into`, the
/// directions into the faces), settled on the surfaces.
fn first_section(
    solver: &Solver<'_>,
    s: f64,
    into: [Vec3; 2],
    edge: EdgeId,
) -> Result<Section, BlendError> {
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    let middle = solver.spine.at(s).ok_or_else(unsolvable)?;
    let sides = solver.sides;
    let mut seed_params = [(0.0, 0.0); 2];
    let mut seed_feet = [middle.point; 2];
    for k in 0..2 {
        let side = sides.get(k).ok_or_else(lost)?;
        let uv = side
            .surface
            .project(middle.point)
            .map_err(|_| unsolvable())?;
        if let Some(p) = seed_params.get_mut(k) {
            *p = uv;
        }
    }
    let size = solver.shape.size();
    for k in 0..2 {
        if let (Some(f), Some(i), Some(p)) =
            (seed_feet.get_mut(k), into.get(k), seed_params.get_mut(k))
        {
            let side = sides.get(k).ok_or_else(lost)?;
            let (uv, foot) = side
                .foot(middle.point + *i * size, *p)
                .ok_or_else(unsolvable)?;
            *f = foot;
            *p = uv;
        }
    }
    let [b1, b2] = into;
    let spread = 1.0 + b1.dot(b2);
    let centre = match solver.shape {
        Shape::Ball { radius, .. } if spread > 1e-9 => {
            let half_cos = (0.5 * spread).sqrt();
            let half_sin = (1.0 - 0.5 * spread).max(0.0).sqrt();
            if !(half_sin > 1e-9) {
                return Err(refused(edge, Reason::Smooth));
            }
            Some(
                middle.point
                    + (b1 + b2).normalized().ok_or_else(unsolvable)?
                        * (radius / half_sin)
                        * half_cos.signum(),
            )
        }
        Shape::Ball { .. } => return Err(refused(edge, Reason::Smooth)),
        Shape::Cut { .. } => None,
    };
    let seed = Section {
        feet: seed_feet,
        params: seed_params,
        centre,
    };
    solver.section(s, &seed).ok_or_else(unsolvable)
}

/// Why a march past the end of an edge stopped.
#[derive(Clone, Copy, Debug)]
enum Stop {
    /// A section could not be found.
    Unsolvable,
    /// The blend went too far past the end without leaving the third
    /// face.
    TooFar,
}

/// Whether the whole of `section` lies outside the face `third`, by more
/// than twice the tolerance.
fn out_of(third: Side<'_>, section: &Section, tol: f64) -> bool {
    beyond_by(third, section, tol, 1.0)
}

/// Whether the whole of `section` lies more than twice the tolerance
/// beyond the face `third`: outside it (`sign` 1), or inside it (−1).
fn beyond_by(third: Side<'_>, section: &Section, tol: f64, sign: f64) -> bool {
    (0..=8).all(|k| {
        section.eval(f64::from(k) / 8.0).is_some_and(|p| {
            third
                .surface
                .project(p)
                .ok()
                .and_then(|uv| third.signed(p, uv))
                .is_some_and(|(d, _)| sign * d > 2.0 * tol)
        })
    })
}

/// Sections from `from` along the edge to its end at `end` (0 or 1) and
/// past it, until a whole section lies outside the third face there;
/// with how far past the end that was.
fn march_out(
    solver: &Solver<'_>,
    from: (f64, Section),
    end: f64,
    third: Side<'_>,
    steps: Steps,
    tol: f64,
) -> Result<(Vec<(f64, Section)>, f64), Stop> {
    let out_of = |section: &Section| out_of(third, section, tol);
    let direction = if end < 0.5 { -1.0 } else { 1.0 };
    let (mut s, mut last) = from;
    let mut solved = Vec::new();
    loop {
        let past = (s - end) * direction;
        let step = if past >= -1e-12 {
            steps.outside
        } else {
            steps.inside
        };
        let next = s + direction * step;
        last = solver.section(next, &last).ok_or(Stop::Unsolvable)?;
        s = next;
        solved.push((s, last));
        let past = (s - end) * direction;
        if past >= -1e-12 && out_of(&last) {
            return Ok((solved, past));
        }
        // No further than the face's approximation can take samples: a
        // blend along an edge that runs nearly along the third face has a
        // long way to go (`End::steps`), and one that never leaves it
        // would go on for ever.
        if past > steps.most_out || solved.len() >= MAX_SAMPLES {
            return Err(Stop::TooFar);
        }
    }
}

/// The cut at one end of a blend face (`last`: the end at `s = 1`), which
/// reaches `past` beyond it: from a few sizes before the end (where the
/// face is still behind the third face's surface near the corner; from
/// further in, a curved third face may be nearer on its other side).
/// `relative` is the blend's size over the edge's length.
fn end_cut(
    surface: &NurbsSurface,
    third: Side<'_>,
    last: bool,
    past: f64,
    relative: f64,
    limit: f64,
    scale: f64,
) -> Option<(NurbsCurve, [f64; 2])> {
    let back = (4.0 * relative).min(0.5);
    let (inner, outer) = if last {
        (1.0 - back, 1.0 + past)
    } else {
        (back, -past)
    };
    cut_curve(surface, third, inner, outer, limit, scale)
}

/// Which way is out of the blend face, at the sample nearest the middle
/// of its domain: away from the ball's centre at a convex edge (towards
/// it at a concave one); for a chamfer towards the edge at a convex one,
/// what is cut away lying there.
fn outward_from(
    solver: &Solver<'_>,
    surface: &NurbsSurface,
    params: &[f64],
    sections: &[Section],
    convex: bool,
) -> Option<Vec3> {
    let ((u0, u1), _) = surface.domain();
    let middle = 0.5 * (u0 + u1);
    let (s, section) = params
        .iter()
        .zip(sections)
        .min_by(|a, b| (a.0 - middle).abs().total_cmp(&(b.0 - middle).abs()))?;
    // The sample's own parameter on the surface (a ring's are taken from
    // its first sample round).
    let u = if *s < u0 { *s + 1.0 } else { *s };
    let p = surface.eval(u, 0.5);
    Some(match (solver.shape, section.centre) {
        (Shape::Ball { .. }, Some(c)) => {
            if convex {
                p - c
            } else {
                c - p
            }
        }
        (Shape::Ball { .. }, None) => return None,
        (Shape::Cut { .. }, _) => {
            let e = solver.spine.at(*s)?.point;
            if convex { e - p } else { p - e }
        }
    })
}

/// The border `border` of a blend face lies on `face` and stays on it:
/// it comes within ten tolerances of no other edge of that face but the
/// ones at its own ends. Proven (`geom::certify::curves_apart`): both
/// curves are cut into pieces held within capsules round their chords (a
/// radius from the bound on their second derivatives), pieces whose
/// capsules are further apart are apart, and the others are halved until
/// they are, or until two points of the curves are seen within ten
/// tolerances (the border meets the edge). A pair of curves that cannot be
/// told apart in the pieces allowed is refused as meeting: never a border
/// taken to stay on its face unproven. (Chords alone, as sampled before,
/// miss crossings on a curved face: each chord cuts inside the surface by
/// its sagitta, two that cross on it pass each other in space by the
/// difference.)
fn stays_on(
    body: &Body,
    edge: EdgeId,
    border: EdgeId,
    face: FaceId,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let tol = tolerance.linear;
    let data = body.edge(border).ok_or_else(lost)?;
    let ordered = |(a, b): (f64, f64)| (a.min(b), a.max(b));
    let mine = CurveBound::new(&data.curve);
    let ends = [data.start, data.end];
    let loops = &body.face(face).ok_or_else(lost)?.loops;
    for l in loops {
        for coedge in &body.loop_(*l).ok_or_else(lost)?.coedges {
            if coedge.edge == border {
                continue;
            }
            let other = body.edge(coedge.edge).ok_or_else(lost)?;
            if ends.contains(&other.start) || ends.contains(&other.end) {
                continue;
            }
            let theirs = CurveBound::new(&other.curve);
            let apart = curves_apart(
                &mine,
                ordered(data.range),
                &theirs,
                ordered(other.range),
                10.0 * tol,
                PROOF_PIECES,
            );
            if apart != Apart::Yes {
                return Err(refuse(edge, Some(face), Reason::FreeRunsOver));
            }
        }
    }
    Ok(())
}

/// A fillet face is tangent to both faces along its borders, measured
/// again on the body (to ten times what the approximation was refined
/// to); a chamfer passes.
fn tangent_or_refused(
    body: &Body,
    edge: EdgeId,
    [first, second]: [FaceId; 2],
    blend_face: FaceId,
    second_border: EdgeId,
    shape: Shape,
    tolerance: &Tolerance,
) -> Result<FaceId, BlendError> {
    tangent_along(
        body,
        edge,
        &[(edge, first), (second_border, second)],
        blend_face,
        shape,
        tolerance,
    )?;
    Ok(blend_face)
}

/// [`tangent_or_refused`] along `borders`, each with the face it lies
/// on: each stays on its face, and a fillet face is tangent to it.
fn tangent_along(
    body: &Body,
    edge: EdgeId,
    borders: &[(EdgeId, FaceId)],
    blend_face: FaceId,
    shape: Shape,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    for (border, face) in borders {
        stays_on(body, edge, *border, *face, tolerance)?;
    }
    if let Shape::Ball { radius, .. } = shape {
        // Tangent to both faces along its borders, measured.
        let limit = 10.0 * bend(tolerance, radius);
        for &(border, face) in borders {
            let data = body.edge(border).ok_or_else(lost)?;
            let (own, other) = (
                body.face(blend_face).ok_or_else(lost)?,
                body.face(face).ok_or_else(lost)?,
            );
            for k in 1..8 {
                let t = data.range.0 + (data.range.1 - data.range.0) * f64::from(k) / 8.0;
                let p = data.curve.eval(t);
                let normal = |f: &craft3d_topo::Face| -> Option<Vec3> {
                    let (u, v) = f.surface.project(p).ok()?;
                    let n = f.surface.normal(u, v)?;
                    Some(if f.reversed { -n } else { n })
                };
                let (Some(a), Some(b)) = (normal(own), normal(other)) else {
                    return Err(refuse(edge, Some(face), Reason::NotTangent));
                };
                if !(a.cross(b).length() <= limit && a.dot(b) > 0.0) {
                    return Err(refuse(edge, Some(face), Reason::NotTangent));
                }
            }
        }
    }
    Ok(())
}

/// The blend face along a closed edge: sections evenly round it (a
/// fillet's then in the planes square to the closed path of their
/// centres), joined by the closed cubic through them (with continuous
/// curvature where they meet), refined as [`refine`] does an open one.
/// With the samples it went through.
type Ring = (NurbsSurface, Vec<f64>, Vec<Section>);

fn ring(solver: &Solver<'_>, seed: Section, data: &Edge, tolerance: &Tolerance) -> Option<Ring> {
    let size = solver.shape.size();
    let mut length = 0.0;
    let mut last = data.curve.eval(data.range.0);
    for k in 1..=128 {
        let t = data.range.0 + (data.range.1 - data.range.0) * f64::from(k) / 128.0;
        let p = data.curve.eval(t);
        length += p.distance(last);
        last = p;
    }
    if !(length > 0.0 && size > 0.0) {
        return None;
    }
    let count = (length / size).ceil().clamp(12.0, 200.0);
    let step = 1.0 / count;
    // Round from the seed at s = 1/2, each section from the one before.
    let mut solved = Vec::new();
    let mut near = seed;
    let mut s = 0.5;
    while s < 1.5 - 0.5 * step {
        near = solver.section(s, &near)?;
        solved.push((s.rem_euclid(1.0), near));
        s += step;
    }
    solved.sort_by(|a, b| a.0.total_cmp(&b.0));
    let limit = 0.25 * tolerance.linear;
    // A fillet's sections are then taken in the planes square to the
    // closed path of the balls' centres (a cubic with continuous
    // curvature through the centres found): the planes across the
    // edge turn only as smoothly as the edge's curve, and an edge that is
    // a fitted curve (where two cylinders meet: a cubic with continuous
    // tangents only) gives planes whose turning jumps at every knot,
    // which the refinement of the face cannot follow. Which planes pick
    // the balls does not change the surface. A chamfer's lines are
    // measured from the edge's point in its plane, so it keeps the
    // edge's planes.
    let path = match solver.shape {
        Shape::Ball { .. } => {
            let balls: Vec<Section> = solved.iter().map(|x| x.1).collect();
            let (curve, at) = closed_path(&balls)?;
            Some((curve, at, balls))
        }
        Shape::Cut { .. } => None,
    };
    let along_path;
    let (solver, mut params, mut sections) = match &path {
        Some((curve, at, balls)) => {
            along_path = Solver {
                spine: Spine {
                    curve,
                    range: (0.0, 1.0),
                    closed: true,
                    joints: [None, None],
                    turn: 0.0,
                },
                sides: solver.sides,
                shape: solver.shape,
                scale: solver.scale,
                moved: None,
            };
            let mut params = Vec::with_capacity(at.len());
            let mut sections = Vec::with_capacity(at.len());
            for (s, near) in at.iter().zip(balls) {
                sections.push(along_path.section(*s, near)?);
                params.push(*s);
            }
            (&along_path, params, sections)
        }
        None => (
            solver,
            solved.iter().map(|x| x.0).collect(),
            solved.iter().map(|x| x.1).collect(),
        ),
    };
    let bend = bend(tolerance, size);
    let surface = refine_samples(solver, &mut params, &mut sections, limit, bend, true)?;
    Some((surface, params, sections))
}

/// The closed path of the centres of `balls` (in order round the edge):
/// the cubic with continuous curvature through them, one Bézier piece
/// per pair, its parameter their distance along the chords from 0 round
/// to 1; with each ball's parameter.
fn closed_path(balls: &[Section]) -> Option<(Curve3, Vec<f64>)> {
    use craft3d_geom::refine::periodic_derivatives;
    let centres: Vec<Vec3> = balls.iter().map(|b| b.centre).collect::<Option<_>>()?;
    let n = centres.len();
    let chords: Vec<f64> = (0..n)
        .map(|i| Some(centres.get(i)?.distance(*centres.get((i + 1) % n)?)))
        .collect::<Option<_>>()?;
    let total: f64 = chords.iter().sum();
    if n < 3 || !(total > 0.0) || chords.iter().any(|c| !(*c > 0.0)) {
        return None;
    }
    let steps: Vec<f64> = chords.iter().map(|c| c / total).collect();
    let mut params = Vec::with_capacity(n);
    let mut sum = 0.0;
    for step in &steps {
        params.push(sum);
        sum += step;
    }
    let values: Vec<Vec<f64>> = centres.iter().map(|c| vec![c.x, c.y, c.z]).collect();
    let derivatives = periodic_derivatives(&steps, &values).ok()?;
    let tangent = |i: usize| -> Option<Vec3> {
        match derivatives.get(i % n)?.as_slice() {
            [x, y, z] => Some(Vec3::new(*x, *y, *z)),
            _ => None,
        }
    };
    let mut ctrl = Vec::with_capacity(3 * n + 1);
    for i in 0..n {
        let (p, q) = (*centres.get(i)?, *centres.get((i + 1) % n)?);
        let third = *steps.get(i)? / 3.0;
        ctrl.push(p);
        ctrl.push(p + tangent(i)? * third);
        ctrl.push(q - tangent(i + 1)? * third);
    }
    ctrl.push(*centres.first()?);
    let mut knots = vec![0.0; 4];
    for p in params.iter().skip(1) {
        knots.extend([*p; 3]);
    }
    knots.extend([1.0; 4]);
    let curve = NurbsCurve::polynomial(3, knots, ctrl).ok()?;
    Some((Curve3::Nurbs(curve), params))
}

/// The closed surface through the sections at `params` (increasing, in
/// [0, 1)): per row of control points the closed cubic with continuous
/// curvature through their homogeneous points, one Bézier piece per pair
/// of sections; its parameter runs from the first section round to it
/// again.
fn ring_surface(params: &[f64], sections: &[Section]) -> Option<NurbsSurface> {
    use craft3d_geom::refine::periodic_derivatives;
    let nets: Vec<Vec<(Vec3, f64)>> = sections
        .iter()
        .map(Section::net)
        .collect::<Option<Vec<_>>>()?;
    let across = nets.first()?.len();
    let n = params.len();
    if n < 3 || nets.iter().any(|x| x.len() != across) {
        return None;
    }
    let first = *params.first()?;
    let steps: Vec<f64> = (0..n)
        .map(|i| {
            let a = params.get(i).copied().unwrap_or(0.0);
            let b = params.get(i + 1).copied().unwrap_or(first + 1.0);
            b - a
        })
        .collect();
    // Per section, its homogeneous control points one after the other.
    let values: Vec<Vec<f64>> = nets
        .iter()
        .map(|net| {
            net.iter()
                .flat_map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
                .collect()
        })
        .collect();
    let tangents = periodic_derivatives(&steps, &values).ok()?;
    let count_u = 3 * n + 1;
    let mut ctrl = vec![Vec3::ZERO; count_u * across];
    let mut weights = vec![0.0; count_u * across];
    for j in 0..across {
        let h = |i: usize| -> Option<(Vec3, f64)> {
            let (p, w) = *nets.get(i % n)?.get(j)?;
            Some((p * w, w))
        };
        let t = |i: usize| -> Option<(Vec3, f64)> {
            match tangents.get(i % n)?.get(4 * j..4 * j + 4)? {
                [x, y, z, w] => Some((Vec3::new(*x, *y, *z), *w)),
                _ => None,
            }
        };
        let mut column = Vec::with_capacity(count_u);
        for i in 0..n {
            let third = *steps.get(i)? / 3.0;
            let (a, ta) = (h(i)?, t(i)?);
            let (b, tb) = (h(i + 1)?, t(i + 1)?);
            column.push(a);
            column.push((a.0 + ta.0 * third, a.1 + ta.1 * third));
            column.push((b.0 - tb.0 * third, b.1 - tb.1 * third));
        }
        column.push(h(0)?);
        for (i, (p, w)) in column.into_iter().enumerate() {
            if !(w > 0.0) {
                return None;
            }
            let index = i * across + j;
            *ctrl.get_mut(index)? = p * (1.0 / w);
            *weights.get_mut(index)? = w;
        }
    }
    let mut knots_u = vec![first; 4];
    for &p in params.iter().skip(1) {
        knots_u.extend([p, p, p]);
    }
    knots_u.extend([first + 1.0; 4]);
    let mut knots_v = vec![0.0; across];
    knots_v.extend(std::iter::repeat_n(1.0, across));
    NurbsSurface::new(
        3,
        across - 1,
        knots_u,
        knots_v,
        count_u,
        across,
        ctrl,
        weights,
    )
    .ok()
}

/// Adds a vertex at `point` to `body`; its id.
fn add_vertex(body: &mut Body, point: Vec3) -> Result<VertexId, BlendError> {
    let id = VertexId(u32::try_from(body.vertices.len()).map_err(|_| lost())?);
    body.vertices.push(Vertex { point });
    Ok(id)
}

/// Adds `edge` to `body`; its id.
fn add_edge(body: &mut Body, edge: Edge) -> Result<EdgeId, BlendError> {
    let id = EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?);
    body.edges.push(edge);
    Ok(id)
}

/// The faces beside a blend once its face is in: `faces[1]` uses
/// `second_border` where it used `edge` (which stays the first border),
/// and both run it the other way round when `turn` (a chain that goes
/// against the edge's curve). How `faces[0]` runs along the first border
/// then (whether its coedge is reversed).
fn hang_borders(
    body: &mut Body,
    edge: EdgeId,
    faces: [FaceId; 2],
    second_border: EdgeId,
    turn: bool,
) -> Result<bool, BlendError> {
    let mut found = None;
    for (k, face) in faces.iter().enumerate() {
        let loops = body.face(*face).ok_or_else(lost)?.loops.clone();
        let mut done = false;
        for l in loops {
            let ring = body.loops.get_mut(l.0 as usize).ok_or_else(lost)?;
            for coedge in &mut ring.coedges {
                if coedge.edge != edge {
                    continue;
                }
                if k == 1 {
                    coedge.edge = second_border;
                }
                if turn {
                    coedge.reversed = !coedge.reversed;
                }
                if k == 0 {
                    found = Some(coedge.reversed);
                }
                done = true;
            }
        }
        if !done {
            return Err(lost());
        }
    }
    found.ok_or_else(lost)
}

/// Adds the blend face on `surface` with the loops `loops` to the shell
/// of `beside`, facing along `outward` at `(u, 0.5)`; its id.
fn add_face(
    body: &mut Body,
    surface: NurbsSurface,
    loops: Vec<Vec<Coedge>>,
    u: f64,
    outward: Vec3,
    beside: FaceId,
) -> Result<FaceId, BlendError> {
    let mut ids = Vec::with_capacity(loops.len());
    for coedges in loops {
        ids.push(LoopId(u32::try_from(body.loops.len()).map_err(|_| lost())?));
        body.loops.push(Loop { coedges });
    }
    let (_, du, dv) = surface.derivatives(u, 0.5);
    let reversed = du.cross(dv).dot(outward) < 0.0;
    let id = FaceId(u32::try_from(body.faces.len()).map_err(|_| lost())?);
    body.faces.push(craft3d_topo::Face {
        surface: Surface::Nurbs(surface),
        reversed,
        loops: ids,
    });
    body.shells
        .iter_mut()
        .find(|s| s.faces.contains(&beside))
        .ok_or_else(lost)?
        .faces
        .push(id);
    Ok(id)
}

/// Puts the blend face of a closed edge into the body: the edge becomes
/// the first border (its vertex moved onto it), the second border is new
/// with a vertex of its own, and the face between them has the two as
/// its loops.
fn splice_ring(
    body: &mut Body,
    edge: EdgeId,
    [first, second]: [FaceId; 2],
    surface: NurbsSurface,
    outward: Vec3,
) -> Result<(FaceId, EdgeId), BlendError> {
    let ((u0, u1), _) = surface.domain();
    let b1 = border(&surface, false).ok_or_else(lost)?;
    let b2 = border(&surface, true).ok_or_else(lost)?;
    let vertex = body.edge(edge).ok_or_else(lost)?.start;
    if let Some(v) = body.vertices.get_mut(vertex.0 as usize) {
        v.point = b1.eval(u0);
    }
    let added = add_vertex(body, b2.eval(u0))?;
    *body.edges.get_mut(edge.0 as usize).ok_or_else(lost)? = Edge {
        curve: Curve3::Nurbs(b1),
        range: (u0, u1),
        start: vertex,
        end: vertex,
    };
    let second_border = add_edge(
        body,
        Edge {
            curve: Curve3::Nurbs(b2),
            range: (u0, u1),
            start: added,
            end: added,
        },
    )?;
    let first_reversed = hang_borders(body, edge, [first, second], second_border, false)?;
    let loops = [(edge, !first_reversed), (second_border, first_reversed)]
        .map(|(edge, reversed)| vec![Coedge { edge, reversed }]);
    let id = add_face(body, surface, loops.into(), 0.5 * (u0 + u1), outward, first)?;
    Ok((id, second_border))
}

/// Puts the blend face into the body in place of the edge (see the
/// module documentation for what changes).
fn splice(
    body: &mut Body,
    edge: EdgeId,
    faces: [FaceId; 2],
    ends: [End; 2],
    made: Made,
    outward: Vec3,
    tolerance: &Tolerance,
) -> Result<(FaceId, EdgeId), BlendError> {
    let tol = tolerance.linear;
    let too_large = |face: FaceId| refuse(edge, Some(face), Reason::FreeTooLarge);
    let Made { surface, cuts } = made;
    let borders = [
        border(&surface, false).ok_or_else(lost)?,
        border(&surface, true).ok_or_else(lost)?,
    ];
    let [(cut0, s0), (cut1, s1)] = &cuts;
    // Per border: the range of s it covers, between the two cuts.
    let spans = [(s0[0], s1[0]), (s0[1], s1[1])];
    for (k, (a, b)) in spans.iter().enumerate() {
        if !(b > a) {
            return Err(too_large(*faces.get(k).ok_or_else(lost)?));
        }
    }
    // The four corners: per end, on the first and on the second face.
    let corner = |end: usize, k: usize| -> Option<Vec3> {
        let s = cuts.get(end)?.1.get(k)?;
        Some(borders.get(k)?.eval(*s))
    };
    // Shorten the other edges at each end; the vertex of the edge goes to
    // the first face's corner, a new vertex for the second face's.
    let mut new_vertex = [VertexId(0); 2];
    let mut own_vertex = [VertexId(0); 2];
    for (index, end) in ends.iter().enumerate() {
        let (Some(p1), Some(p2)) = (corner(index, 0), corner(index, 1)) else {
            return Err(lost());
        };
        let [own, added] = cut_back(body, end, faces, [p1, p2], tol, &too_large)?;
        if let (Some(a), Some(b)) = (new_vertex.get_mut(index), own_vertex.get_mut(index)) {
            *a = added;
            *b = own;
        }
    }
    let [start_vertex, end_vertex] = own_vertex;
    // The edge becomes the first border; the second border and the two
    // cuts are new.
    {
        let slot = body.edges.get_mut(edge.0 as usize).ok_or_else(lost)?;
        let [b1, _] = &borders;
        *slot = Edge {
            curve: Curve3::Nurbs(b1.clone()),
            range: spans[0],
            start: start_vertex,
            end: end_vertex,
        };
    }
    let second_border = add_edge(
        body,
        Edge {
            curve: Curve3::Nurbs(borders[1].clone()),
            range: spans[1],
            start: new_vertex[0],
            end: new_vertex[1],
        },
    )?;
    let mut cut_edges = [EdgeId(0); 2];
    for (index, curve) in [cut0, cut1].into_iter().enumerate() {
        let (from, to) = (
            *[start_vertex, end_vertex].get(index).ok_or_else(lost)?,
            *new_vertex.get(index).ok_or_else(lost)?,
        );
        let id = add_edge(
            body,
            Edge {
                curve: Curve3::Nurbs(curve.clone()),
                range: (0.0, 1.0),
                start: from,
                end: to,
            },
        )?;
        if let Some(slot) = cut_edges.get_mut(index) {
            *slot = id;
        }
    }
    let first_reversed = hang_borders(body, edge, faces, second_border, false)?;
    // The third faces: the cut in place of the corner.
    for (index, end) in ends.iter().enumerate() {
        let cut = *cut_edges.get(index).ok_or_else(lost)?;
        let corners = [
            *own_vertex.get(index).ok_or_else(lost)?,
            *new_vertex.get(index).ok_or_else(lost)?,
        ];
        insert_cut(body, end, cut, corners)?;
    }
    let [cut0, cut1] = cut_edges;
    let coedges = blend_loop(
        edge,
        second_border,
        first_reversed,
        &[(cut0, false)],
        &[(cut1, false)],
    );
    let [first, _] = faces;
    let id = add_face(
        body,
        surface,
        vec![coedges],
        0.5 * (spans[0].0 + spans[0].1),
        outward,
        first,
    )?;
    Ok((id, second_border))
}

/// The loop of a blend face: along its first border against the way the
/// first face runs it (`first_reversed`: how the first face's coedge
/// runs), then from the first border's corner to the second's at the end
/// it has come to (`finish` or `start`: edges each with whether it runs
/// that way against its curve), back along the second border, and across
/// the other end.
fn blend_loop(
    first_border: EdgeId,
    second_border: EdgeId,
    first_reversed: bool,
    start: &[(EdgeId, bool)],
    finish: &[(EdgeId, bool)],
) -> Vec<Coedge> {
    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let forwards = |list: &[(EdgeId, bool)]| -> Vec<Coedge> {
        list.iter().map(|(e, r)| coedge(*e, *r)).collect()
    };
    let backwards = |list: &[(EdgeId, bool)]| -> Vec<Coedge> {
        list.iter().rev().map(|(e, r)| coedge(*e, !*r)).collect()
    };
    let mut coedges = Vec::with_capacity(2 + start.len() + finish.len());
    if first_reversed {
        // The first face runs the border backwards: this face forwards.
        coedges.push(coedge(first_border, false));
        coedges.extend(forwards(finish));
        coedges.push(coedge(second_border, true));
        coedges.extend(backwards(start));
    } else {
        coedges.push(coedge(first_border, true));
        coedges.extend(forwards(start));
        coedges.push(coedge(second_border, false));
        coedges.extend(backwards(finish));
    }
    coedges
}

/// At one end of a blend: moves the vertex of the edge to `corners[0]`
/// (where the first border ends), adds a vertex at `corners[1]` (the
/// second border's end), and shortens the two other edges there to the
/// corners; returns the vertices of the two corners. A corner that is
/// the far end of the other edge on its side (a border that runs exactly
/// into the corner where that edge turns: a second chamfer of the size
/// of an earlier one beside it) is that end's vertex, and the other edge,
/// taken whole, leaves the body (its vertex there with it on the first
/// side); [`compact`] drops what no longer has a use.
fn cut_back(
    body: &mut Body,
    end: &End,
    faces: [FaceId; 2],
    corners: [Vec3; 2],
    tol: f64,
    too_large: &dyn Fn(FaceId) -> BlendError,
) -> Result<[VertexId; 2], BlendError> {
    let [first_corner, second_corner] = corners;
    let mut found = [end.vertex, end.vertex];
    for (k, (other, point)) in end.others.iter().zip(corners).enumerate() {
        let face = *faces.get(k).ok_or_else(lost)?;
        let data = body.edge(*other).ok_or_else(lost)?.clone();
        let far = if data.start == end.vertex {
            data.end
        } else {
            data.start
        };
        let far_point = body.vertex(far).ok_or_else(lost)?.point;
        if far != end.vertex && far_point.distance(point) <= tol {
            take_whole(body, *other);
            if let Some(slot) = found.get_mut(k) {
                *slot = far;
            }
            continue;
        }
        let Ok(on_other) = parameter_on(&data, point, tol) else {
            return Err(too_large(face));
        };
        // Not on the edge itself: past the vertices at which the boundary
        // between this face and the third goes on between the same two
        // faces (a split of one curve of contact, such as where a
        // cylinder's parameter closes), the edges passed taken whole.
        let along = match on_other {
            Some(_) => None,
            None => walk_on(
                body,
                *other,
                end.vertex,
                [face, end.third],
                end.across.get(k).copied().flatten(),
                point,
                tol,
            ),
        };
        if let Some((passed, at)) = along {
            for e in &passed {
                take_whole(body, *e);
            }
            match at {
                Along::At(vertex) => {
                    if let Some(slot) = found.get_mut(k) {
                        *slot = vertex;
                    }
                    continue;
                }
                Along::On {
                    edge: next,
                    near,
                    t,
                } => {
                    let target = corner_vertex(body, end, k, second_corner)?;
                    if let Some(slot) = found.get_mut(k) {
                        *slot = target;
                    }
                    shorten(body, next, near, target, t)?;
                    continue;
                }
            }
        }
        let target = corner_vertex(body, end, k, second_corner)?;
        if let Some(slot) = found.get_mut(k) {
            *slot = target;
        }
        if let Some(t) = on_other {
            shorten(body, *other, end.vertex, target, t)?;
            continue;
        }
        let (lo, hi) = data.range;
        // Past the vertex (a blend that adds material, at a concave edge,
        // ending where the faces beside it meet the third face at convex
        // edges, as a blade's root fillet at the blade's end): the edge
        // keeps all it had, and a new edge goes on from its end to the
        // corner along both its faces (a line between two planes; any other
        // curve through points where the faces cross, held to both between
        // them), the vertex between the two left where it was.
        let at_start = data.start == end.vertex;
        let (vertex, d) = data.curve.derivatives(if at_start { lo } else { hi });
        let out = (if at_start { -d } else { d })
            .normalized()
            .ok_or_else(lost)?;
        if !((point - vertex).dot(out) > tol) {
            return Err(too_large(face));
        }
        let (curve, range) =
            going_on(body, vertex, point, [face, end.third], tol).ok_or_else(|| too_large(face))?;
        let middle = add_vertex(body, vertex)?;
        let slot = body.edges.get_mut(other.0 as usize).ok_or_else(lost)?;
        if at_start {
            slot.start = middle;
        } else {
            slot.end = middle;
        }
        let on = add_edge(
            body,
            Edge {
                curve,
                range,
                start: middle,
                end: target,
            },
        )?;
        for f in [face, end.third] {
            insert_next_to(body, f, *other, on, middle)?;
            let lengthened = body.edge(on).ok_or_else(lost)?.clone();
            // The face's surface has to reach where the edge now goes.
            widen_to(body, f, &lengthened, tol).ok_or_else(|| too_large(face))?;
        }
    }
    if found[0] == end.vertex
        && let Some(v) = body.vertices.get_mut(end.vertex.0 as usize)
    {
        v.point = first_corner;
    }
    Ok(found)
}

/// Takes `edge` whole out of the body: out of every loop, and nothing
/// ends at it any more ([`compact`] drops it and the vertices only it
/// had).
fn take_whole(body: &mut Body, edge: EdgeId) {
    for ring in &mut body.loops {
        ring.coedges.retain(|c| c.edge != edge);
    }
    if let Some(slot) = body.edges.get_mut(edge.0 as usize) {
        slot.start = VertexId(u32::MAX);
        slot.end = VertexId(u32::MAX);
    }
}

/// The parameter of `point` on the edge `data` when it lies on it within
/// `tol`, strictly between its ends; `Err` when it cannot be projected.
fn parameter_on(data: &Edge, point: Vec3, tol: f64) -> Result<Option<f64>, ()> {
    let t = data.curve.project(point).map_err(|_| ())?;
    let (lo, hi) = data.range;
    // A periodic curve: the turn that puts `t` in the range.
    let t = match data.curve.period() {
        Some(period) if period > 0.0 => {
            let mut t = t;
            while t < lo - tol {
                t += period;
            }
            while t > hi + tol {
                t -= period;
            }
            t
        }
        _ => t,
    };
    let span = hi - lo;
    let on =
        data.curve.eval(t).distance(point) <= tol && t > lo + 1e-9 * span && t < hi - 1e-9 * span;
    Ok(on.then_some(t))
}

/// The vertex of a corner at one end of a blend: the end's own vertex on
/// the first side (moved there once both sides are done), a new one at
/// `point` on the second.
fn corner_vertex(
    body: &mut Body,
    end: &End,
    side: usize,
    point: Vec3,
) -> Result<VertexId, BlendError> {
    if side == 0 {
        return Ok(end.vertex);
    }
    add_vertex(body, point)
}

/// Shortens `edge` at its end at `near` to the parameter `t`, where it
/// now ends at `target`.
fn shorten(
    body: &mut Body,
    edge: EdgeId,
    near: VertexId,
    target: VertexId,
    t: f64,
) -> Result<(), BlendError> {
    let slot = body.edges.get_mut(edge.0 as usize).ok_or_else(lost)?;
    if slot.start == near {
        slot.range.0 = t;
        slot.start = target;
    } else if slot.end == near {
        slot.range.1 = t;
        slot.end = target;
    } else {
        return Err(lost());
    }
    Ok(())
}

/// Where a corner of a blend lies on the boundary between a face and the
/// third face, past the edge it starts from.
#[derive(Clone, Copy, Debug)]
enum Along {
    /// At this vertex of the boundary.
    At(VertexId),
    /// On `edge`, at `t`; `near` is its end towards the blend.
    On {
        edge: EdgeId,
        near: VertexId,
        t: f64,
    },
}

/// The corner `point` on the boundary between `faces` (a face beside the
/// edge and the third face), followed from `other` (which leaves the
/// blended edge's vertex `from` along it) across every vertex at which
/// that boundary goes on between the same two faces and nothing else
/// meets: a vertex of those two edges only, a split of one curve of
/// contact (where the parameter of a cylinder or a revolved face closes,
/// or where an earlier operation cut the curve) that bounds nothing of
/// either face. And across the vertex where `across` (a smooth edge of the
/// face, which the blend's border crosses: [`cross`]) leaves the
/// boundary: from there on the boundary is the one between the face
/// beyond that edge and the third face. With the edges passed, which the
/// blend takes whole. `None` where the boundary turns into another face,
/// or ends, before the corner is reached.
fn walk_on(
    body: &Body,
    other: EdgeId,
    from: VertexId,
    [face, third]: [FaceId; 2],
    across: Option<EdgeId>,
    point: Vec3,
    tol: f64,
) -> Option<(Vec<EdgeId>, Along)> {
    let pair = |e: EdgeId| {
        let mut f = faces_of_edge(body, e);
        f.sort();
        f
    };
    let sorted = |a: FaceId, b: FaceId| if a <= b { vec![a, b] } else { vec![b, a] };
    let mut want = sorted(face, third);
    let mut crossed = false;
    let mut passed = Vec::new();
    let mut current = other;
    let mut near = from;
    // Every edge at most once.
    for _ in 0..body.edges.len() {
        let data = body.edge(current)?;
        let far = if data.start == near {
            data.end
        } else if data.end == near {
            data.start
        } else {
            return None;
        };
        if far == near || far == from {
            return None;
        }
        let at: Vec<EdgeId> = body
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.start == far || e.end == far)
            .map(|(i, _)| EdgeId(u32::try_from(i).unwrap_or(u32::MAX)))
            .collect();
        let rest: Vec<EdgeId> = at.iter().copied().filter(|e| *e != current).collect();
        let next = match rest.as_slice() {
            [next] => *next,
            // The smooth edge leaves here: the boundary goes on between
            // the face beyond it and the third face.
            [a, b] if !crossed && across.is_some_and(|g| g == *a || g == *b) => {
                let (g, next) = if across == Some(*a) {
                    (*a, *b)
                } else {
                    (*b, *a)
                };
                let beyond = match faces_of_edge(body, g).as_slice() {
                    [x, y] if *x == face && *y != face => *y,
                    [x, y] if *y == face && *x != face => *x,
                    _ => return None,
                };
                want = sorted(beyond, third);
                crossed = true;
                next
            }
            _ => return None,
        };
        if next == other || passed.contains(&next) || pair(next) != want {
            return None;
        }
        passed.push(current);
        let next_data = body.edge(next)?;
        if next_data.start == next_data.end {
            return None;
        }
        let next_far = if next_data.start == far {
            next_data.end
        } else {
            next_data.start
        };
        if body.vertex(next_far)?.point.distance(point) <= tol {
            passed.push(next);
            return Some((passed, Along::At(next_far)));
        }
        if let Ok(Some(t)) = parameter_on(next_data, point, tol) {
            return Some((
                passed,
                Along::On {
                    edge: next,
                    near: far,
                    t,
                },
            ));
        }
        current = next;
        near = far;
    }
    None
}

/// The curve from `from` to `to` along where the surfaces of `faces`
/// cross, with its range: a line where both are planes (exact), otherwise
/// a cubic through points of both (alternating projections onto each,
/// continued past its patch), refined until every point halfway between
/// two of them is within a quarter of the tolerance of both.
fn going_on(
    body: &Body,
    from: Vec3,
    to: Vec3,
    faces: [FaceId; 2],
    tol: f64,
) -> Option<(Curve3, (f64, f64))> {
    let [fa, fb] = faces;
    let surfaces = [&body.face(fa)?.surface, &body.face(fb)?.surface];
    let limit = 0.25 * tol;
    let near = |p: Vec3| {
        surfaces
            .iter()
            .all(|s| foot_past(s, p).is_some_and(|f| f.distance(p) <= limit))
    };
    if !(near(from) && near(to)) {
        return None;
    }
    if surfaces.iter().all(|s| matches!(s, Surface::Plane(_))) {
        let line = craft3d_geom::Line::through(from, to).ok()?;
        let curve = Curve3::Line(line);
        let range = (curve.project(from).ok()?, curve.project(to).ok()?);
        return Some((curve, range));
    }
    let mut n = 4_u32;
    for _ in 0..8 {
        let mut points = vec![from];
        for j in 1..n {
            let guess = from + (to - from) * (f64::from(j) / f64::from(n));
            points.push(onto_both(surfaces, guess, limit)?);
        }
        points.push(to);
        let mut params = vec![0.0];
        for pair in points.windows(2) {
            let [a, b] = pair else { return None };
            let last = *params.last()?;
            params.push(last + a.distance(*b));
        }
        let total = *params.last()?;
        if !(total > 0.0) {
            return None;
        }
        for p in &mut params {
            *p /= total;
        }
        let curve = curve_through(&params, &points)?;
        // At the middles, and proven between them on both surfaces.
        let proven = || {
            surfaces.iter().all(|s| {
                s.project(from).is_ok_and(|start| {
                    proven_on(&curve, (0.0, 1.0), s, start, &params, limit).is_ok()
                })
            })
        };
        if params
            .windows(2)
            .all(|w| matches!(w, [a, b] if near(curve.eval(0.5 * (a + b)))))
            && proven()
        {
            return Some((Curve3::Nurbs(curve), (0.0, 1.0)));
        }
        n *= 2;
    }
    None
}

/// Puts `on` (an edge that goes on from `edge` at `at`) into the loops of
/// `face` next to `edge`: after it where the loop arrives at `at` along
/// it, before it where the loop leaves `at` along it.
fn insert_next_to(
    body: &mut Body,
    face: FaceId,
    edge: EdgeId,
    on: EdgeId,
    at: VertexId,
) -> Result<(), BlendError> {
    let (start, end) = {
        let data = body.edge(edge).ok_or_else(lost)?;
        (data.start, data.end)
    };
    let on_starts_at = body.edge(on).ok_or_else(lost)?.start == at;
    let loops = body.face(face).ok_or_else(lost)?.loops.clone();
    for l in loops {
        let ring = body.loops.get(l.0 as usize).ok_or_else(lost)?.clone();
        let Some(index) = ring.coedges.iter().position(|c| c.edge == edge) else {
            continue;
        };
        let here = *ring.coedges.get(index).ok_or_else(lost)?;
        let arrives = if here.reversed { start } else { end };
        let mut coedges = ring.coedges.clone();
        if arrives == at {
            // Leaving `at` along `on`.
            coedges.insert(
                index + 1,
                Coedge {
                    edge: on,
                    reversed: !on_starts_at,
                },
            );
        } else {
            // Arriving at `at` along `on`.
            coedges.insert(
                index,
                Coedge {
                    edge: on,
                    reversed: on_starts_at,
                },
            );
        }
        if let Some(slot) = body.loops.get_mut(l.0 as usize) {
            slot.coedges = coedges;
        }
        return Ok(());
    }
    Err(lost())
}

/// Grows the spline surface of `face` past its domain (continued as the
/// function it is, so nothing of the face moves) until it takes in the
/// edge `edge` lies along it: where a blend lengthens an edge beyond the
/// patch the face was made on. Analytic surfaces have no bounds to grow.
fn widen_to(body: &mut Body, face: FaceId, edge: &Edge, tol: f64) -> Option<()> {
    let data = body.face(face)?;
    let nurbs = match &data.surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok()?,
        _ => return Some(()),
    };
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    let (mut lo_u, mut hi_u, mut lo_v, mut hi_v) = (u0, u1, v0, v1);
    let side = Side {
        surface: &data.surface,
        reversed: false,
    };
    for j in 0..=32 {
        let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(j) / 32.0;
        let p = edge.curve.eval(t);
        let start = data.surface.project(p).ok()?;
        let ((u, v), foot) = side.foot(p, start)?;
        // Only as far as the edge reaches along the surface continued: a
        // foot that is not the edge's point says the surface does not go
        // on there as the edge does.
        if !(foot.distance(p) <= tol) {
            return None;
        }
        lo_u = lo_u.min(u);
        hi_u = hi_u.max(u);
        lo_v = lo_v.min(v);
        hi_v = hi_v.max(v);
    }
    if (lo_u, hi_u, lo_v, hi_v) == (u0, u1, v0, v1) {
        return Some(());
    }
    let wider = nurbs.extended((lo_u, hi_u), (lo_v, hi_v), tol).ok()?;
    body.faces.get_mut(face.0 as usize)?.surface = Surface::Nurbs(wider);
    Some(())
}

/// The nearest point to `p` on both `surfaces` (where they cross), by
/// alternating projections; `None` unless it lies within `limit` of both.
fn onto_both(surfaces: [&Surface; 2], p: Vec3, limit: f64) -> Option<Vec3> {
    let mut q = p;
    for _ in 0..200 {
        let before = q;
        for s in surfaces {
            q = foot_past(s, q)?;
        }
        if q.distance(before) <= 1e-3 * limit {
            break;
        }
    }
    surfaces
        .iter()
        .all(|s| foot_past(s, q).is_some_and(|f| f.distance(q) <= limit))
        .then_some(q)
}

/// The nearest point to `p` of `surface`, a spline surface continued past
/// its domain as the polynomials its end spans are (where a face's edge is
/// lengthened beyond it).
fn foot_past(surface: &Surface, p: Vec3) -> Option<Vec3> {
    let start = surface.project(p).ok()?;
    let side = Side {
        surface,
        reversed: false,
    };
    side.foot(p, start).map(|(_, f)| f)
}

/// Puts the cut `cut` (from the first face's corner to the second face's,
/// the vertices `[own, added]`) into the third face at `end`, in place of
/// its corner.
fn insert_cut(
    body: &mut Body,
    end: &End,
    cut: EdgeId,
    [own, added]: [VertexId; 2],
) -> Result<(), BlendError> {
    let loops = body.face(end.third).ok_or_else(lost)?.loops.clone();
    let mut done = false;
    for l in loops {
        let ring = body.loops.get(l.0 as usize).ok_or_else(lost)?.clone();
        let mut coedges = Vec::with_capacity(ring.coedges.len() + 1);
        for coedge in &ring.coedges {
            coedges.push(*coedge);
            let data = body.edge(coedge.edge).ok_or_else(lost)?;
            let arrives = if coedge.reversed {
                data.start
            } else {
                data.end
            };
            // The coedge that arrives at a corner (the edge that was there,
            // or what `cut_back` made go on from it: a loop passes a
            // vertex once).
            if !done && arrives == own {
                // From the first face's corner to the second's.
                coedges.push(Coedge {
                    edge: cut,
                    reversed: false,
                });
                done = true;
            } else if !done && arrives == added {
                coedges.push(Coedge {
                    edge: cut,
                    reversed: true,
                });
                done = true;
            }
        }
        if let Some(slot) = body.loops.get_mut(l.0 as usize) {
            slot.coedges = coedges;
        }
    }
    if done { Ok(()) } else { Err(lost()) }
}

/// Every loop of every face still lies where it bounds its face
/// (`craft3d_topo::stranded`): a blend built in place cuts its faces back
/// (the two beside the edge, the third faces at its ends) and knows only
/// the loops it changes, so a hole or a boss wholly inside what it takes
/// is left outside its face, a body every other check passes. Refused as
/// reaching that face, which `fit` reads as no room.
fn no_stranded_loop(body: &Body, edge: EdgeId, tolerance: &Tolerance) -> Result<(), BlendError> {
    match craft3d_topo::stranded(body, tolerance).first() {
        Some((face, _)) => Err(refuse(edge, Some(*face), Reason::Reaches)),
        None => Ok(()),
    }
}

/// No loop of a face next to the blend face `blend` (the faces beside its
/// edge, the third faces at its ends) lies in what the blend took: the
/// section between its two borders and the original edge `original`. A
/// point is in it when its nearest point on the blend's surface lies
/// inside the sections (`v` strictly between the borders, `u` inside the
/// surface's domain), it is on the side of the surface that was removed
/// (outside at a convex edge) or filled (inside at a concave one), and it
/// is no further from that nearest point than the section's diameter: the
/// largest side of the triangle of the two borders' points there and the
/// edge's nearest point (the section lies within that triangle's hull with
/// the arc, whatever the angle between the faces; the chord alone does
/// not bound it at a sharp edge). Points of every loop that shares no
/// edge and no vertex with the blend face are looked at, spread over the
/// whole loop: vertices and middles of its edges. A point near the blend
/// that cannot be projected onto its surface cannot be shown clear, and
/// the blend is refused, never passed. This holds for faces of any
/// surface, as [`no_stranded_loop`] does only on planes so far. Refused as
/// reaching that face.
fn no_loop_behind(
    body: &Body,
    blend: FaceId,
    convex: bool,
    original: &Curve3,
    edge: EdgeId,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    // `v` and `u` this close to the ends of their ranges are on a border:
    // relative to the parameter range, scale-free.
    const EDGE_V: f64 = 1e-6;
    // Points looked at per loop, spread over it.
    const MOST_POINTS: usize = 64;
    let tol = tolerance.linear;
    let face = body.face(blend).ok_or_else(lost)?;
    let Surface::Nurbs(surface) = &face.surface else {
        return Ok(());
    };
    let mut own_edges = std::collections::BTreeSet::new();
    let mut own_vertices = std::collections::BTreeSet::new();
    for l in &face.loops {
        for coedge in &body.loop_(*l).ok_or_else(lost)?.coedges {
            own_edges.insert(coedge.edge);
            if let Some(data) = body.edge(coedge.edge) {
                own_vertices.insert(data.start);
                own_vertices.insert(data.end);
            }
        }
    }
    // What the blend took lies within the hull of its surface's control
    // net and the original edge's points at its sections (each section is
    // within the triangle of its two border points and the edge's point,
    // with its arc): the box of those, grown by ten tolerances.
    let mut low = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut high = -low;
    let mut grow = |p: Vec3| {
        low = Vec3::new(low.x.min(p.x), low.y.min(p.y), low.z.min(p.z));
        high = Vec3::new(high.x.max(p.x), high.y.max(p.y), high.z.max(p.z));
    };
    for p in surface.control_points() {
        grow(*p);
    }
    let ((u0, u1), _) = surface.domain();
    for k in 0..=32 {
        let u = u0 + (u1 - u0) * f64::from(k) / 32.0;
        let middle = surface.eval(u, 0.5);
        if let Ok(t) = original.project(middle) {
            grow(original.eval(t));
        }
    }
    let slack = Vec3::new(10.0 * tol, 10.0 * tol, 10.0 * tol);
    let (low, high) = (low - slack, high + slack);
    let in_box = |p: Vec3| {
        p.x >= low.x
            && p.x <= high.x
            && p.y >= low.y
            && p.y <= high.y
            && p.z >= low.z
            && p.z <= high.z
    };
    let whole = Surface::Nurbs(surface.clone());
    for (index, other) in body.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(index).map_err(|_| lost())?);
        if id == blend {
            continue;
        }
        let rings: Vec<&Loop> = other.loops.iter().filter_map(|l| body.loop_(*l)).collect();
        let beside = rings
            .iter()
            .any(|r| r.coedges.iter().any(|c| own_edges.contains(&c.edge)));
        if !beside {
            continue;
        }
        for ring in rings {
            let touches = ring.coedges.iter().any(|c| {
                own_edges.contains(&c.edge)
                    || body.edge(c.edge).is_some_and(|d| {
                        own_vertices.contains(&d.start) || own_vertices.contains(&d.end)
                    })
            });
            if touches {
                continue;
            }
            // Two points per coedge, every `stride`-th coedge: spread
            // over the whole loop.
            let stride = ring.coedges.len().div_ceil(MOST_POINTS / 2).max(1);
            let mut points = Vec::new();
            for coedge in ring.coedges.iter().step_by(stride) {
                let data = body.edge(coedge.edge).ok_or_else(lost)?;
                points.push(body.vertex(data.start).ok_or_else(lost)?.point);
                points.push(data.curve.eval(0.5 * (data.range.0 + data.range.1)));
            }
            for p in points.into_iter().filter(|p| in_box(*p)) {
                let Ok((u, v)) = whole.project(p) else {
                    return Err(refuse(edge, Some(id), Reason::Reaches));
                };
                let span = u1 - u0;
                if !(v > EDGE_V
                    && v < 1.0 - EDGE_V
                    && u > u0 + EDGE_V * span
                    && u < u1 - EDGE_V * span)
                {
                    continue;
                }
                let (q, du, dv) = surface.derivatives(u, v);
                let Some(n) = du.cross(dv).normalized() else {
                    return Err(refuse(edge, Some(id), Reason::Reaches));
                };
                let n = if face.reversed { -n } else { n };
                let (b0, b1) = (surface.eval(u, 0.0), surface.eval(u, 1.0));
                let Ok(t) = original.project(q) else {
                    return Err(refuse(edge, Some(id), Reason::Reaches));
                };
                let e = original.eval(t);
                let diameter = b0.distance(b1).max(b0.distance(e)).max(b1.distance(e));
                if p.distance(q) > diameter {
                    continue;
                }
                let side = (p - q).dot(n);
                if (convex && side > tol) || (!convex && side < -tol) {
                    return Err(refuse(edge, Some(id), Reason::Reaches));
                }
            }
        }
    }
    Ok(())
}

/// [`no_loop_behind`] for every blend face `made` of a chain or of meets,
/// each against its own edge as it was in `before`.
fn behind_all(
    before: &Body,
    out: &Body,
    made: &[(FaceId, usize)],
    items: &[Item],
    edge: EdgeId,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    for (face, index) in made {
        let item = items.get(*index).ok_or_else(lost)?;
        let original = &before.edge(item.request.edge).ok_or_else(lost)?.curve;
        no_loop_behind(out, *face, item.convex, original, edge, tolerance)?;
    }
    Ok(())
}

/// Drops the edges no loop uses and the vertices no edge has (what a
/// blend took whole, see [`cut_back`]), keeping the order of the rest;
/// per edge and per vertex left, the index it had.
fn compact(body: &mut Body) -> (Vec<usize>, Vec<usize>) {
    let mut used_edge = vec![false; body.edges.len()];
    for ring in &body.loops {
        for coedge in &ring.coedges {
            if let Some(flag) = used_edge.get_mut(coedge.edge.0 as usize) {
                *flag = true;
            }
        }
    }
    let edge_was: Vec<usize> = (0..body.edges.len())
        .filter(|i| used_edge.get(*i).copied().unwrap_or(false))
        .collect();
    let mut used_vertex = vec![false; body.vertices.len()];
    for &i in &edge_was {
        if let Some(edge) = body.edges.get(i) {
            for v in [edge.start, edge.end] {
                if let Some(flag) = used_vertex.get_mut(v.0 as usize) {
                    *flag = true;
                }
            }
        }
    }
    let vertex_was: Vec<usize> = (0..body.vertices.len())
        .filter(|i| used_vertex.get(*i).copied().unwrap_or(false))
        .collect();
    if edge_was.len() == body.edges.len() && vertex_was.len() == body.vertices.len() {
        return (edge_was, vertex_was);
    }
    let mut edge_now = vec![u32::MAX; body.edges.len()];
    for (now, &was) in edge_was.iter().enumerate() {
        if let Some(slot) = edge_now.get_mut(was) {
            *slot = u32::try_from(now).unwrap_or(u32::MAX);
        }
    }
    let mut vertex_now = vec![u32::MAX; body.vertices.len()];
    for (now, &was) in vertex_was.iter().enumerate() {
        if let Some(slot) = vertex_now.get_mut(was) {
            *slot = u32::try_from(now).unwrap_or(u32::MAX);
        }
    }
    let vertex_id =
        |v: VertexId| VertexId(vertex_now.get(v.0 as usize).copied().unwrap_or(u32::MAX));
    body.edges = edge_was
        .iter()
        .filter_map(|&i| body.edges.get(i).cloned())
        .map(|mut edge| {
            edge.start = vertex_id(edge.start);
            edge.end = vertex_id(edge.end);
            edge
        })
        .collect();
    body.vertices = vertex_was
        .iter()
        .filter_map(|&i| body.vertices.get(i).cloned())
        .collect();
    for ring in &mut body.loops {
        for coedge in &mut ring.coedges {
            coedge.edge = EdgeId(
                edge_now
                    .get(coedge.edge.0 as usize)
                    .copied()
                    .unwrap_or(u32::MAX),
            );
        }
    }
    (edge_was, vertex_was)
}

/// Drops the faces no shell has (a face a blend took whole, see
/// [`cross`]) and the loops no face has, keeping the order of the rest;
/// per face left, the index it had.
fn compact_faces(body: &mut Body) -> Vec<usize> {
    let mut kept = vec![false; body.faces.len()];
    for shell in &body.shells {
        for face in &shell.faces {
            if let Some(flag) = kept.get_mut(face.0 as usize) {
                *flag = true;
            }
        }
    }
    let face_was: Vec<usize> = (0..body.faces.len())
        .filter(|i| kept.get(*i).copied().unwrap_or(false))
        .collect();
    if face_was.len() == body.faces.len() {
        return face_was;
    }
    let mut face_now = vec![u32::MAX; body.faces.len()];
    for (now, &was) in face_was.iter().enumerate() {
        if let Some(slot) = face_now.get_mut(was) {
            *slot = u32::try_from(now).unwrap_or(u32::MAX);
        }
    }
    body.faces = face_was
        .iter()
        .filter_map(|&i| body.faces.get(i).cloned())
        .collect();
    for shell in &mut body.shells {
        for face in &mut shell.faces {
            *face = FaceId(face_now.get(face.0 as usize).copied().unwrap_or(u32::MAX));
        }
    }
    // The loops the faces left have, in their order.
    let mut used = vec![false; body.loops.len()];
    for face in &body.faces {
        for l in &face.loops {
            if let Some(flag) = used.get_mut(l.0 as usize) {
                *flag = true;
            }
        }
    }
    let loop_was: Vec<usize> = (0..body.loops.len())
        .filter(|i| used.get(*i).copied().unwrap_or(false))
        .collect();
    let mut loop_now = vec![u32::MAX; body.loops.len()];
    for (now, &was) in loop_was.iter().enumerate() {
        if let Some(slot) = loop_now.get_mut(was) {
            *slot = u32::try_from(now).unwrap_or(u32::MAX);
        }
    }
    body.loops = loop_was
        .iter()
        .filter_map(|&i| body.loops.get(i).cloned())
        .collect();
    for face in &mut body.faces {
        for l in &mut face.loops {
            *l = LoopId(loop_now.get(l.0 as usize).copied().unwrap_or(u32::MAX));
        }
    }
    face_was
}

/// One edge to blend, read from the body: its request, its two faces
/// (the first one first), the directions into them and whether it is
/// convex.
#[derive(Clone, Copy, Debug)]
struct Item {
    request: Request,
    faces: [FaceId; 2],
    into: [Vec3; 2],
    convex: bool,
}

/// True when `plan` of [`super`] refused `reason` for a kind of edge
/// this module may build.
pub(super) fn takes(reason: Reason) -> bool {
    matches!(
        reason,
        Reason::KindLine | Reason::KindCircle | Reason::KindEllipse | Reason::KindSpline
    )
}

/// Blends `requests` of `body`, none of whose edges the closed forms
/// cover, one after the other in place.
pub(super) fn blend(
    body: &Body,
    requests: &[Request],
    tolerance: &Tolerance,
) -> Result<Blended, BlendError> {
    if body.kind != BodyKind::Solid {
        return Err(BlendError::Refused {
            edge: requests.first().map(|r| r.edge),
            face: None,
            reason: Reason::NotSolid,
        });
    }
    let uses = super::edge_uses(body);
    let mut items = Vec::with_capacity(requests.len());
    for request in requests {
        let (faces, into, convex) = super::oriented(body, &uses, request)?;
        items.push(Item {
            request: *request,
            faces,
            into,
            convex,
        });
    }
    // Edges that go on into each other are blended as one chain; no
    // other two may share a vertex.
    let (chains, meets) = chain::group(body, &uses, &items, tolerance)?;
    let mut out = body.clone();
    let mut blends = Vec::with_capacity(requests.len());
    let mut with_meets = Vec::new();
    for (index, found) in chains.iter().enumerate() {
        if found.meets.iter().any(Option::is_some) {
            with_meets.push(index);
            continue;
        }
        let first = found.pieces.first().map_or(EdgeId(u32::MAX), |p| p.edge);
        if let ([piece], []) = (found.pieces.as_slice(), found.joints.as_slice()) {
            // Held to `no_loop_behind` inside, on its fitted surface.
            let item = items.get(piece.item).ok_or_else(lost)?;
            let face = blend_one(
                &mut out,
                &item.request,
                item.faces,
                item.into,
                item.convex,
                tolerance,
            )?;
            blends.push((face, piece.item));
        } else {
            let made = chain::build(&mut out, found, &items, tolerance)?;
            behind_all(body, &out, &made, &items, first, tolerance)?;
            blends.extend(made);
        }
        no_stranded_loop(&out, first, tolerance)?;
    }
    if !with_meets.is_empty() {
        let made = chain::build_meets(&mut out, &chains, &with_meets, &meets, &items, tolerance)?;
        let first = with_meets
            .first()
            .and_then(|c| chains.get(*c))
            .and_then(|c| c.pieces.first())
            .map_or(EdgeId(u32::MAX), |p| p.edge);
        behind_all(body, &out, &made, &items, first, tolerance)?;
        blends.extend(made);
        no_stranded_loop(&out, first, tolerance)?;
    }
    let mut seen = std::collections::BTreeSet::new();
    for request in requests {
        if let Some(data) = body.edge(request.edge) {
            seen.insert(data.start);
            seen.insert(data.end);
        }
    }
    let (edge_was, vertex_was) = compact(&mut out);
    let face_was = compact_faces(&mut out);
    let mut defects = craft3d_topo::check(&out, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(BlendError::Kernel(KernelError::InvalidBody {
            operation: "a blend along a freeform edge",
            defects,
        }));
    }
    // Where everything came from.
    let mut faces: Vec<Face> = (0..body.faces.len())
        .map(|i| Face::Kept(vec![FaceId(u32::try_from(i).unwrap_or(u32::MAX))]))
        .collect();
    let made_faces = face_was.iter().copied().max().map_or(0, |m| m + 1);
    faces.resize(made_faces.max(faces.len()), Face::Kept(Vec::new()));
    for (face, index) in &blends {
        if let Some(slot) = faces.get_mut(face.0 as usize) {
            *slot = Face::Blend(vec![*index]);
        }
    }
    // A face a border carried across a smooth edge took whole is gone.
    let faces: Vec<Face> = face_was
        .iter()
        .map(|&i| faces.get(i).cloned().unwrap_or(Face::Kept(Vec::new())))
        .collect();
    let replaced: std::collections::BTreeSet<EdgeId> = requests.iter().map(|r| r.edge).collect();
    let edges = edge_was
        .iter()
        .map(|&i| {
            let id = EdgeId(u32::try_from(i).unwrap_or(u32::MAX));
            (i < body.edges.len() && !replaced.contains(&id)).then_some(id)
        })
        .collect();
    let vertices = vertex_was
        .iter()
        .map(|&i| {
            let id = VertexId(u32::try_from(i).unwrap_or(u32::MAX));
            (i < body.vertices.len() && !seen.contains(&id)).then_some(id)
        })
        .collect();
    Ok(Blended {
        body: out,
        faces,
        edges,
        vertices,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use craft3d_geom::{Line, Plane};
    use craft3d_math::Frame;

    /// A blend past the end of its edge that never leaves the third face
    /// (here a plane it never reaches, as an edge running along the face
    /// would be): with no limit on how far it may go, the march stops at
    /// the most samples the face can take, instead of going on for ever.
    #[test]
    fn a_march_that_never_leaves_the_third_face_stops_at_the_sample_cap() {
        let top = Surface::Plane(Plane::new(Frame::WORLD));
        let side = Surface::Plane(Plane::new(
            Frame::new(Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0), Vec3::X).unwrap(),
        ));
        let away = Surface::Plane(Plane::new(
            Frame::new(
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::X,
            )
            .unwrap(),
        ));
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap());
        let solver = Solver {
            spine: Spine {
                curve: &line,
                range: (0.0, 1.0),
                closed: false,
                joints: [None, None],
                turn: 0.0,
            },
            sides: [
                Side {
                    surface: &top,
                    reversed: false,
                },
                Side {
                    surface: &side,
                    reversed: false,
                },
            ],
            shape: Shape::Ball {
                radius: 0.1,
                convex: true,
            },
            scale: 1.0,
            moved: None,
        };
        let seed = Section {
            feet: [Vec3::new(0.5, 0.1, 0.0), Vec3::new(0.5, 0.0, -0.1)],
            params: [
                top.project(Vec3::new(0.5, 0.1, 0.0)).unwrap(),
                side.project(Vec3::new(0.5, 0.0, -0.1)).unwrap(),
            ],
            centre: Some(Vec3::new(0.5, 0.1, -0.1)),
        };
        let seed = solver.section(0.5, &seed).expect("the seed ball");
        let steps = Steps {
            inside: 0.01,
            outside: 0.01,
            most_out: f64::INFINITY,
        };
        let third = Side {
            surface: &away,
            reversed: false,
        };
        let stopped = march_out(&solver, (0.5, seed), 1.0, third, steps, 1e-6);
        assert!(matches!(stopped, Err(Stop::TooFar)));
    }

    /// A wall across a flat blend face, waving along it with nodes at
    /// every sixteenth of the face's width: where the cut is first fitted
    /// (eight steps) and at the middles between, the wall is where a
    /// straight cut would be; between them it is `amplitude` off it.
    fn waving_wall(amplitude: f64, k: f64) -> (NurbsSurface, Surface) {
        let flat = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, k, 0.0),
                Vec3::new(k, 0.0, 0.0),
                Vec3::new(k, k, 0.0),
            ],
            vec![1.0; 4],
        )
        .unwrap();
        // x = k / 2 + amplitude sin(16 π y / k), interpolated at every
        // 1/128 of y; straight up and down in z.
        let params: Vec<f64> = (0..=128).map(|i| f64::from(i) / 128.0).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|a| {
                let x = 0.5 * k + amplitude * k * (16.0 * std::f64::consts::PI * a).sin();
                vec![x, a * k]
            })
            .collect();
        let (knots, rows) = interpolate(&params, 3, &values).unwrap();
        let mut ctrl = Vec::new();
        for r in &rows {
            for z in [-0.5 * k, 0.5 * k] {
                ctrl.push(Vec3::new(r[0], r[1], z));
            }
        }
        let count = rows.len();
        let wall = NurbsSurface::new(
            3,
            1,
            knots,
            vec![0.0, 0.0, 1.0, 1.0],
            count,
            2,
            ctrl,
            vec![1.0; 2 * count],
        )
        .unwrap();
        (flat, Surface::Nurbs(wall))
    }

    /// The cut where a blend face crosses the third face is fitted to
    /// points found exactly and held to a quarter of the tolerance at the
    /// middles between them; a third face that waves between those
    /// middles (here a wall a hundred quarter-tolerances off the straight
    /// line through them) is now followed, the cut proven on it.
    #[test]
    fn a_cut_curve_that_would_leave_the_third_face_between_its_middles_is_refined() {
        for k in [1e-3, 1.0, 1e3] {
            let limit = 0.25e-6 * k;
            let (flat, wall) = waving_wall(1e-4, k);
            let third = Side {
                surface: &wall,
                reversed: false,
            };
            let (curve, _) = cut_curve(&flat, third, 0.1, 0.9, limit, k).expect("a cut");
            let start = wall.project(curve.eval(0.0)).unwrap();
            let mut hint = start;
            for i in 0..=2000 {
                let t = f64::from(i) / 2000.0;
                let p = curve.eval(t);
                hint = wall.project_from(p, hint).unwrap();
                let d = wall.eval(hint.0, hint.1).distance(p);
                assert!(d <= limit, "{k}: {d:e} off the wall at {t}");
            }
            // And proven so.
            let bound = SurfaceBound::new(&wall).unwrap();
            let curve = CurveBound::nurbs(&curve);
            assert!(
                bound
                    .curve_within(&curve, (0.0, 1.0), start, &[], limit, 1 << 16)
                    .is_ok()
            );
        }
    }

    /// A flat face with a border that bows down towards another edge of
    /// it, its nearest point `gap` from it between the samples a polyline
    /// of the border would take.
    fn bowed_border(gap: f64, k: f64) -> Body {
        let params: Vec<f64> = (0..=40).map(|i| f64::from(i) / 40.0).collect();
        let points: Vec<Vec3> = params
            .iter()
            .map(|t| Vec3::new(t * k, (gap + 4e-2 * (t - 0.507).powi(2)) * k, 0.0))
            .collect();
        let bow = curve_through(&params, &points).unwrap();
        let line = Line::new(Vec3::new(-0.2 * k, 0.0, 0.0), Vec3::X).unwrap();
        let mut body = Body::new(BodyKind::Sheet);
        for p in [
            *points.first().unwrap(),
            *points.last().unwrap(),
            Vec3::new(-0.2 * k, 0.0, 0.0),
            Vec3::new(1.2 * k, 0.0, 0.0),
        ] {
            body.vertices.push(Vertex { point: p });
        }
        body.edges.push(Edge {
            curve: Curve3::Nurbs(bow),
            range: (0.0, 1.0),
            start: VertexId(0),
            end: VertexId(1),
        });
        body.edges.push(Edge {
            curve: Curve3::Line(line),
            range: (0.0, 1.4 * k),
            start: VertexId(2),
            end: VertexId(3),
        });
        body.loops.push(Loop {
            coedges: vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: true,
                },
            ],
        });
        body.faces.push(craft3d_topo::Face {
            surface: Surface::Plane(Plane::new(Frame::WORLD)),
            reversed: false,
            loops: vec![LoopId(0)],
        });
        body
    }

    /// A border is held apart from the face's other edges by proof: one
    /// that comes within ten tolerances of another edge between the
    /// samples is refused, one that keeps twenty away is not.
    #[test]
    fn a_border_that_comes_near_another_edge_between_samples_is_refused() {
        for k in [1e-3, 1.0, 1e3] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            for (gap, refused) in [(5e-6, true), (9e-6, true), (2e-5, false)] {
                let body = bowed_border(gap, k);
                let found = stays_on(&body, EdgeId(0), EdgeId(0), FaceId(0), &tolerance);
                assert_eq!(found.is_err(), refused, "{k} {gap}");
            }
        }
    }

    /// The new edge from a vertex to a corner is fitted through points
    /// where two faces meet and held to them at the middles; where the
    /// faces meet in a curve that waves between those middles (a flat face
    /// and a wall waving with nodes at every eighth) it is now refined
    /// until it is proven on both.
    #[test]
    fn a_new_edge_that_would_leave_its_faces_between_its_middles_is_refined() {
        for k in [1e-3, 1.0, 1e3] {
            let tol = 1e-6 * k;
            let params: Vec<f64> = (0..=128).map(|i| f64::from(i) / 128.0).collect();
            let values: Vec<Vec<f64>> = params
                .iter()
                .map(|a| {
                    let x = 0.5 * k + 1e-4 * k * (8.0 * std::f64::consts::PI * a).sin();
                    vec![x, a * k]
                })
                .collect();
            let (knots, rows) = interpolate(&params, 3, &values).unwrap();
            let mut ctrl = Vec::new();
            for r in &rows {
                for z in [-0.5 * k, 0.5 * k] {
                    ctrl.push(Vec3::new(r[0], r[1], z));
                }
            }
            let count = rows.len();
            let wall = NurbsSurface::new(
                3,
                1,
                knots,
                vec![0.0, 0.0, 1.0, 1.0],
                count,
                2,
                ctrl,
                vec![1.0; 2 * count],
            )
            .unwrap();
            let mut body = Body::new(BodyKind::Sheet);
            for surface in [
                Surface::Plane(Plane::new(Frame::WORLD)),
                Surface::Nurbs(wall),
            ] {
                body.faces.push(craft3d_topo::Face {
                    surface,
                    reversed: false,
                    loops: Vec::new(),
                });
            }
            let (from, to) = (Vec3::new(0.5 * k, 0.0, 0.0), Vec3::new(0.5 * k, k, 0.0));
            let (curve, range) =
                going_on(&body, from, to, [FaceId(0), FaceId(1)], tol).expect("an edge");
            let wall = &body.faces[1].surface;
            let mut hint = (0.0, 0.5);
            for i in 0..=2000 {
                let t = range.0 + (range.1 - range.0) * f64::from(i) / 2000.0;
                let p = curve.eval(t);
                hint = wall.project_from(p, hint).unwrap();
                let d = wall.eval(hint.0, hint.1).distance(p);
                assert!(d <= 0.25 * tol, "{k}: {d:e} off the wall at {t}");
                assert!(p.z.abs() <= 0.25 * tol);
            }
        }
    }

    /// A fillet face is held against the exact sections at the middles
    /// between its samples; along a wall with bumps between those middles
    /// (flat at every sixteenth, where the samples and middles are), its
    /// border would leave the wall between them (seen: 1.2 times the
    /// limit). It is now proven on the wall, refined until it is.
    #[test]
    fn a_fillet_border_that_would_leave_its_face_between_the_middles_is_refined() {
        let top = Surface::Plane(Plane::new(Frame::WORLD));
        // Cubic pieces a thirty-second wide, flat at both ends: from 0 up
        // to 1e-4 and back down, so the wall is at y = 0, square to y, at
        // every sixteenth.
        let pieces = 32;
        let mut knots = vec![0.0; 4];
        let mut ys = vec![0.0];
        for k in 0..pieces {
            let (from, to) = if k % 2 == 0 { (0.0, 1e-4) } else { (1e-4, 0.0) };
            ys.extend([from, to, to]);
            let end = f64::from(k + 1) / f64::from(pieces);
            knots.extend(if k + 1 == pieces {
                vec![end; 4]
            } else {
                vec![end; 3]
            });
        }
        let count = ys.len();
        let mut ctrl = Vec::new();
        for (i, y) in ys.iter().enumerate() {
            let x = i as f64 / (count - 1) as f64;
            for z in [-1.0, 0.5] {
                ctrl.push(Vec3::new(x, *y, z));
            }
        }
        let wall = Surface::Nurbs(
            NurbsSurface::new(
                3,
                1,
                knots,
                vec![0.0, 0.0, 1.0, 1.0],
                count,
                2,
                ctrl,
                vec![1.0; 2 * count],
            )
            .unwrap(),
        );
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap());
        let solver = Solver {
            spine: Spine {
                curve: &line,
                range: (0.0, 1.0),
                closed: false,
                joints: [None, None],
                turn: 0.0,
            },
            sides: [
                Side {
                    surface: &top,
                    reversed: false,
                },
                Side {
                    surface: &wall,
                    reversed: false,
                },
            ],
            shape: Shape::Ball {
                radius: 0.1,
                convex: true,
            },
            scale: 1.0,
            moved: None,
        };
        let seed = Section {
            feet: [Vec3::new(0.5, 0.1, 0.0), Vec3::new(0.5, 0.0, -0.1)],
            params: [
                top.project(Vec3::new(0.5, 0.1, 0.0)).unwrap(),
                wall.project(Vec3::new(0.5, 0.0, -0.1)).unwrap(),
            ],
            centre: Some(Vec3::new(0.5, 0.1, -0.1)),
        };
        let seed = solver.section(0.5, &seed).expect("the seed ball");
        let mut samples: Vec<f64> = (0..=8).map(|i| f64::from(i) / 8.0).collect();
        let mut sections: Vec<Section> = samples
            .iter()
            .map(|s| solver.follow(0.5, &seed, *s, FOLLOW_DEPTH).expect("a ball"))
            .collect();
        let limit = 0.25e-6;
        let surface =
            refine(&solver, &mut samples, &mut sections, limit, 1e-3).expect("a fillet face");
        let on_wall = border(&surface, true).unwrap();
        // The wall as the blend sees it: continued past its domain.
        let side = solver.sides[1];
        let mut hint = wall.project(on_wall.eval(0.0)).unwrap();
        for i in 0..=4000 {
            let s = f64::from(i) / 4000.0;
            let p = on_wall.eval(s);
            let (uv, foot) = side.foot(p, hint).unwrap();
            hint = uv;
            let d = foot.distance(p);
            assert!(d <= limit, "{d:e} off the wall at {s}");
        }
    }
}
