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
//! face and the faces beside it share their edges exactly.
//!
//! **Where it ends.** An edge that ends in a vertex of exactly three
//! faces (the two of the edge and a third one across it): the blend face
//! is continued past the vertex until it is wholly out of the third face,
//! and cut where it crosses that face's surface, along a curve fitted to
//! points found exactly (to a quarter of the tolerance between them as
//! well). The two other edges at the vertex are shortened to where the
//! borders of the blend reach them, the third face gets the cut curve in
//! place of the corner, and the edge becomes the blend face.
//!
//! **A closed edge** (one vertex no other edge passes through) has no
//! ends: the sections go round it, the surface is the closed cubic through
//! them, and the blend face is the ring between its two borders.
//!
//! **What is refused** (with the reason): an edge that ends in a vertex of
//! more than three faces, or where another blended edge ends; a closed
//! edge whose vertex other edges pass through; a chamfer given by an
//! angle; a blend whose
//! borders leave the edges at its ends (too large); a surface that cannot
//! be followed (a ball that does not settle, an arc of half a turn or
//! more); an approximation that does not come within the tolerance in the
//! samples allowed. The body is changed in place (every other entity
//! keeps its number), passes `craft3d_topo::check` and the tangency test
//! of [`super`]; whether a large blend runs into a face further away is
//! not seen here: the caller looks for places where the body passes
//! through itself, as for a loft.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{BlendError, Blended, Face, Reason, Request, Size, lost, refused};
use crate::KernelError;
use crate::chamfer::ChamferSize;
use craft3d_geom::refine::interpolation;
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Edge, EdgeId, FaceId, Loop, LoopId, Vertex, VertexId};

/// Most samples along one blend face, and across one cut at its end.
const MAX_SAMPLES: usize = 256;
/// Steps of the search for one ball, and of the search for a nearest point.
const BALL_STEPS: usize = 100;
const FOOT_STEPS: usize = 60;
/// How far a fillet face may turn away from the faces it touches, at
/// least: what the approximation is refined to between samples. It may
/// turn by the document's angular tolerance, or by the angle the linear
/// tolerance makes over the radius, whichever is more: a surface within
/// the linear tolerance of the true one can lean by that much.
const BEND: f64 = 1e-9;

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

impl Side<'_> {
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
/// 1 at its end, and along the curve continued past either end.
struct Spine<'a> {
    curve: &'a Curve3,
    range: (f64, f64),
    /// A closed edge: `s` goes round, a whole turn per unit.
    closed: bool,
}

impl Spine<'_> {
    fn at(&self, s: f64) -> Option<Across> {
        let (t0, t1) = self.range;
        // Past the ends the curve goes on as it is (a spline's end spans
        // as the polynomials they are), so the planes turn smoothly.
        let s = if self.closed { s.rem_euclid(1.0) } else { s };
        let t = t0 + s * (t1 - t0);
        let (point, d) = self.curve.derivatives(t);
        let normal = (d * (t1 - t0)).normalized()?;
        point.is_finite().then_some(Across { point, normal })
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
#[derive(Clone, Copy)]
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
}

impl Solver<'_> {
    /// The section in the plane at `s`, started from `near` (a section
    /// nearby).
    fn section(&self, s: f64, near: &Section) -> Option<Section> {
        let across = self.spine.at(s)?;
        match self.shape {
            Shape::Ball { radius, convex } => self.ball(across, near, radius, convex),
            Shape::Cut { distances } => self.cut(across, near, distances),
        }
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
    let (knots_u, matrix) = interpolation(params, degree).ok()?;
    let count_u = params.len();
    let mut ctrl = Vec::with_capacity(count_u * across);
    let mut weights = Vec::with_capacity(count_u * across);
    for row in &matrix {
        for j in 0..across {
            let mut point = Vec3::ZERO;
            let mut weight = 0.0;
            for (m, net) in row.iter().zip(&nets) {
                let (p, w) = *net.get(j)?;
                point += p * (m * w);
                weight += m * w;
            }
            if !(weight > 0.0) {
                return None;
            }
            ctrl.push(point * (1.0 / weight));
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
    let (knots, matrix) = interpolation(params, degree).ok()?;
    let ctrl: Vec<Vec3> = matrix
        .iter()
        .map(|row| {
            row.iter()
                .zip(points)
                .fold(Vec3::ZERO, |sum, (m, p)| sum + *p * *m)
        })
        .collect();
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

fn end_at(
    body: &Body,
    edge: EdgeId,
    faces: [FaceId; 2],
    vertex: VertexId,
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
    Ok(End {
        vertex,
        others: [ox, oy],
        third: t1,
    })
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
    for _ in 0..16 {
        let surface = surface_through(params, sections)?;
        let mut added: Vec<(f64, Section)> = Vec::new();
        for (pair, near) in params.windows(2).zip(sections.iter()) {
            let [a, b] = pair else { continue };
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
            }
            if !(worst <= limit) || !(kink <= bend) {
                added.push((s, exact));
            }
        }
        if added.is_empty() {
            return Some(surface);
        }
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
        if (hi - lo).abs() <= 1e-15 * (1.0 + mid.abs()) || glo.abs() <= 1e-15 * scale {
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
    let face = |f: FaceId| body.face(f).ok_or_else(lost);
    let (fa, fb) = (face(first)?, face(second)?);
    let sides = [
        Side {
            surface: &fa.surface,
            reversed: fa.reversed,
        },
        Side {
            surface: &fb.surface,
            reversed: fb.reversed,
        },
    ];
    let start = data.curve.eval(data.range.0);
    let finish = data.curve.eval(data.range.1);
    let scale = start
        .length()
        .max(finish.length())
        .max(shape.size())
        .max(tol);
    let solver = Solver {
        spine: Spine {
            curve: &data.curve,
            range: data.range,
            closed,
        },
        sides,
        shape,
        scale,
    };
    let unsolvable = || refuse(edge, None, Reason::FreeSurface);
    // The first section, from the edge's middle and the flat estimate.
    let middle = solver.spine.at(0.5).ok_or_else(unsolvable)?;
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
    let size = shape.size();
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
    let centre = match shape {
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
    let seed = solver.section(0.5, &seed).ok_or_else(unsolvable)?;
    if closed {
        let (surface, params, sections) = ring(&solver, seed, &data, tolerance)
            .ok_or_else(|| refuse(edge, None, Reason::FreeApproximation))?;
        let outward =
            outward_from(&solver, &surface, &params, &sections, convex).ok_or_else(unsolvable)?;
        let (blend_face, second_border) =
            splice_ring(body, edge, [first, second], surface, outward)?;
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
    let ends = [
        end_at(body, edge, [first, second], data.start)?,
        end_at(body, edge, [first, second], data.end)?,
    ];
    // Sections from the middle outwards, each from its neighbour: along
    // the edge in steps of a few radii, past its ends in steps of a
    // quarter of the radius until the section is wholly out of the third
    // face there.
    let length = start.distance(finish).max(
        data.curve
            .eval(0.5 * (data.range.0 + data.range.1))
            .distance(start),
    );
    if !(length > 0.0) {
        return Err(unsolvable());
    }
    // One step along the edge and past it, so the samples are evenly
    // spaced (an interpolating spline overshoots where they are not).
    let inside_step = (size / length).clamp(1.0 / 200.0, 1.0 / 24.0);
    let outside_step = inside_step.min(0.5 * size / length);
    let most_out = 8.0 * size / length;
    let thirds: Vec<Side<'_>> = ends
        .iter()
        .map(|end| {
            face(end.third).map(|f| Side {
                surface: &f.surface,
                reversed: f.reversed,
            })
        })
        .collect::<Result<_, _>>()?;
    let out_of = |third: &Side<'_>, section: &Section| -> bool {
        (0..=8).all(|k| {
            section.eval(f64::from(k) / 8.0).is_some_and(|p| {
                third
                    .surface
                    .project(p)
                    .ok()
                    .and_then(|uv| third.signed(p, uv))
                    .is_some_and(|(d, _)| d > 2.0 * tol)
            })
        })
    };
    let mut solved: Vec<(f64, Section)> = vec![(0.5, seed)];
    let mut reach = [0.0_f64; 2];
    for (index, direction) in [(0_usize, -1.0_f64), (1, 1.0)] {
        let third = thirds.get(index).ok_or_else(lost)?;
        let mut last = seed;
        let mut s = 0.5;
        let edge_end = if direction < 0.0 { 0.0 } else { 1.0 };
        loop {
            let past = (s - edge_end) * direction;
            let step = if past >= -1e-12 {
                outside_step
            } else {
                inside_step
            };
            let next = s + direction * step;
            last = solver.section(next, &last).ok_or_else(unsolvable)?;
            s = next;
            solved.push((s, last));
            let past = (s - edge_end) * direction;
            if past >= -1e-12 && out_of(third, &last) {
                if let Some(slot) = reach.get_mut(index) {
                    *slot = past;
                }
                break;
            }
            if past > most_out {
                return Err(refuse(
                    edge,
                    ends.get(index).map(|e| e.third),
                    Reason::FreeEnd,
                ));
            }
        }
    }
    solved.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut params: Vec<f64> = solved.iter().map(|x| x.0).collect();
    let mut sections: Vec<Section> = solved.iter().map(|x| x.1).collect();
    let limit = 0.25 * tol;
    let bend = tolerance.angular.max(BEND).max(tol / size);
    let surface = refine(&solver, &mut params, &mut sections, limit, bend)
        .ok_or_else(|| refuse(edge, None, Reason::FreeApproximation))?;
    // The cuts at the ends.
    let mut cuts = Vec::with_capacity(2);
    for (index, end) in ends.iter().enumerate() {
        let third = face(end.third)?;
        let side = Side {
            surface: &third.surface,
            reversed: third.reversed,
        };
        // From a few radii before the end, where the face is still behind
        // the third face's surface near the corner (from further in, a
        // curved third face may be nearer on its other side).
        let back = (4.0 * size / length).min(0.5);
        let (inner, outer) = if index == 0 {
            (back, -reach[0])
        } else {
            (1.0 - back, 1.0 + reach[1])
        };
        let cut = cut_curve(&surface, side, inner, outer, limit, scale)
            .ok_or_else(|| refuse(edge, Some(end.third), Reason::FreeEnd))?;
        cuts.push(cut);
    }
    let [cut0, cut1] = <[(NurbsCurve, [f64; 2]); 2]>::try_from(cuts).map_err(|_| lost())?;
    let made = Made {
        surface,
        cuts: [cut0, cut1],
    };
    let outward_hint =
        outward_from(&solver, &made.surface, &params, &sections, convex).ok_or_else(unsolvable)?;
    let (blend_face, second_border) = splice(
        body,
        edge,
        [first, second],
        ends,
        made,
        outward_hint,
        tolerance,
    )?;
    tangent_or_refused(
        body,
        edge,
        [first, second],
        blend_face,
        second_border,
        shape,
        tolerance,
    )
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
    let tol = tolerance.linear;
    if let Shape::Ball { radius, .. } = shape {
        // Tangent to both faces along its borders, measured.
        let limit = 10.0 * tolerance.angular.max(BEND).max(tol / radius);
        for (border, face) in [(edge, first), (second_border, second)] {
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
    Ok(blend_face)
}

/// The blend face along a closed edge: sections evenly round it, joined
/// by the closed cubic through them (with continuous curvature where
/// they meet), refined as [`refine`] does an open one. With the samples
/// it went through.
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
    let mut params: Vec<f64> = solved.iter().map(|x| x.0).collect();
    let mut sections: Vec<Section> = solved.iter().map(|x| x.1).collect();
    let limit = 0.25 * tolerance.linear;
    let bend = tolerance.angular.max(BEND).max(tolerance.linear / size);
    for _ in 0..16 {
        let surface = ring_surface(&params, &sections)?;
        let first = *params.first()?;
        let mut added: Vec<(f64, Section)> = Vec::new();
        for (i, near) in sections.iter().enumerate() {
            let a = *params.get(i)?;
            let b = params.get(i + 1).copied().unwrap_or(first + 1.0);
            let s = 0.5 * (a + b);
            let exact = solver.section(s, near)?;
            let mut worst = 0.0_f64;
            for k in 0..=8 {
                let v = f64::from(k) / 8.0;
                worst = worst.max(surface.eval(s, v).distance(exact.eval(v)?));
            }
            let mut kink = 0.0_f64;
            if matches!(solver.shape, Shape::Ball { .. }) {
                for (k, v) in [(0_usize, 0.0), (1, 1.0)] {
                    let (point, du, dv) = surface.derivatives(s, v);
                    let side = solver.sides.get(k)?;
                    let (uv, _) = side.foot(point, *exact.params.get(k)?)?;
                    let n = side.normal(uv)?;
                    kink = kink.max(du.cross(dv).normalized()?.cross(n).length());
                }
            }
            if !(worst <= limit) || !(kink <= bend) {
                added.push((s.rem_euclid(1.0), exact));
            }
        }
        if added.is_empty() {
            return Some((surface, params, sections));
        }
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

/// The closed surface through the sections at `params` (increasing, in
/// [0, 1)): per row of control points the closed cubic with continuous
/// curvature through their homogeneous points, one Bézier piece per pair
/// of sections; its parameter runs from the first section round to it
/// again.
fn ring_surface(params: &[f64], sections: &[Section]) -> Option<NurbsSurface> {
    use craft3d_geom::refine::periodic_tangents;
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
    let tangents = periodic_tangents(&steps).ok()?;
    let count_u = 3 * n + 1;
    let mut ctrl = vec![Vec3::ZERO; count_u * across];
    let mut weights = vec![0.0; count_u * across];
    for j in 0..across {
        let h = |i: usize| -> Option<(Vec3, f64)> {
            let (p, w) = *nets.get(i % n)?.get(j)?;
            Some((p * w, w))
        };
        let t = |i: usize| -> Option<(Vec3, f64)> {
            let row = tangents.get(i % n)?;
            let mut sum = (Vec3::ZERO, 0.0);
            for (k, m) in row.iter().enumerate() {
                let (p, w) = h(k)?;
                sum.0 += p * *m;
                sum.1 += w * m;
            }
            Some(sum)
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
    let added = VertexId(u32::try_from(body.vertices.len()).map_err(|_| lost())?);
    body.vertices.push(Vertex { point: b2.eval(u0) });
    *body.edges.get_mut(edge.0 as usize).ok_or_else(lost)? = Edge {
        curve: Curve3::Nurbs(b1),
        range: (u0, u1),
        start: vertex,
        end: vertex,
    };
    let second_border = EdgeId(u32::try_from(body.edges.len()).map_err(|_| lost())?);
    body.edges.push(Edge {
        curve: Curve3::Nurbs(b2),
        range: (u0, u1),
        start: added,
        end: added,
    });
    let mut first_reversed = None;
    for (face, replacement) in [(first, edge), (second, second_border)] {
        let loops = body.face(face).ok_or_else(lost)?.loops.clone();
        for l in loops {
            let ring = body.loops.get_mut(l.0 as usize).ok_or_else(lost)?;
            for coedge in &mut ring.coedges {
                if coedge.edge == edge {
                    coedge.edge = replacement;
                    if face == first {
                        first_reversed = Some(coedge.reversed);
                    }
                }
            }
        }
    }
    let first_reversed = first_reversed.ok_or_else(lost)?;
    let mut ids = Vec::with_capacity(2);
    for (edge, reversed) in [(edge, !first_reversed), (second_border, first_reversed)] {
        ids.push(LoopId(u32::try_from(body.loops.len()).map_err(|_| lost())?));
        body.loops.push(Loop {
            coedges: vec![Coedge { edge, reversed }],
        });
    }
    let (_, du, dv) = surface.derivatives(0.5 * (u0 + u1), 0.5);
    let reversed = du.cross(dv).dot(outward) < 0.0;
    let id = FaceId(u32::try_from(body.faces.len()).map_err(|_| lost())?);
    body.faces.push(craft3d_topo::Face {
        surface: Surface::Nurbs(surface),
        reversed,
        loops: ids,
    });
    body.shells
        .iter_mut()
        .find(|s| s.faces.contains(&first))
        .ok_or_else(lost)?
        .faces
        .push(id);
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
    for (index, end) in ends.iter().enumerate() {
        let (Some(p1), Some(p2)) = (corner(index, 0), corner(index, 1)) else {
            return Err(lost());
        };
        let added = VertexId(u32::try_from(body.vertices.len()).map_err(|_| lost())?);
        body.vertices.push(Vertex { point: p2 });
        if let Some(slot) = new_vertex.get_mut(index) {
            *slot = added;
        }
        for (k, (other, point)) in end.others.iter().zip([p1, p2]).enumerate() {
            let face = *faces.get(k).ok_or_else(lost)?;
            let target = if k == 0 { end.vertex } else { added };
            let data = body.edge(*other).ok_or_else(lost)?.clone();
            let t = data.curve.project(point).map_err(|_| too_large(face))?;
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
            if !(data.curve.eval(t).distance(point) <= tol) {
                return Err(too_large(face));
            }
            let slot = body.edges.get_mut(other.0 as usize).ok_or_else(lost)?;
            let span = hi - lo;
            if slot.start == end.vertex {
                if !(t > lo && t < hi - 1e-9 * span) {
                    return Err(too_large(face));
                }
                slot.range.0 = t;
                slot.start = target;
            } else if slot.end == end.vertex {
                if !(t < hi && t > lo + 1e-9 * span) {
                    return Err(too_large(face));
                }
                slot.range.1 = t;
                slot.end = target;
            } else {
                return Err(lost());
            }
        }
        if let Some(v) = body.vertices.get_mut(end.vertex.0 as usize) {
            v.point = p1;
        }
    }
    let [start_vertex, end_vertex] = [ends[0].vertex, ends[1].vertex];
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
    let next_edge = |body: &Body| EdgeId(u32::try_from(body.edges.len()).unwrap_or(u32::MAX));
    let second_border = next_edge(body);
    body.edges.push(Edge {
        curve: Curve3::Nurbs(borders[1].clone()),
        range: spans[1],
        start: new_vertex[0],
        end: new_vertex[1],
    });
    let mut cut_edges = [EdgeId(0); 2];
    for (index, curve) in [cut0, cut1].into_iter().enumerate() {
        let id = next_edge(body);
        let (from, to) = (
            *[start_vertex, end_vertex].get(index).ok_or_else(lost)?,
            *new_vertex.get(index).ok_or_else(lost)?,
        );
        body.edges.push(Edge {
            curve: Curve3::Nurbs(curve.clone()),
            range: (0.0, 1.0),
            start: from,
            end: to,
        });
        if let Some(slot) = cut_edges.get_mut(index) {
            *slot = id;
        }
    }
    // The second face uses the second border where it used the edge.
    let [first, second] = faces;
    let mut first_reversed = None;
    for (face, replacement) in [(first, edge), (second, second_border)] {
        let loops = body.face(face).ok_or_else(lost)?.loops.clone();
        let mut done = false;
        for l in loops {
            let ring = body.loops.get_mut(l.0 as usize).ok_or_else(lost)?;
            for coedge in &mut ring.coedges {
                if coedge.edge == edge {
                    coedge.edge = replacement;
                    if face == first {
                        first_reversed = Some(coedge.reversed);
                    }
                    done = true;
                }
            }
        }
        if !done {
            return Err(lost());
        }
    }
    let first_reversed = first_reversed.ok_or_else(lost)?;
    // The third faces: the cut in place of the corner.
    for (index, end) in ends.iter().enumerate() {
        let cut = *cut_edges.get(index).ok_or_else(lost)?;
        let [to_first, to_second] = end.others;
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
                if coedge.edge == to_first && arrives == end.vertex {
                    // From the first face's corner to the second's.
                    coedges.push(Coedge {
                        edge: cut,
                        reversed: false,
                    });
                    done = true;
                } else if coedge.edge == to_second
                    && arrives == *new_vertex.get(index).ok_or_else(lost)?
                {
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
        if !done {
            return Err(lost());
        }
    }
    // The blend face: against the first face along the first border.
    let coedges = if first_reversed {
        // The first face runs the border backwards: this face forwards.
        vec![
            Coedge {
                edge,
                reversed: false,
            },
            Coedge {
                edge: cut_edges[1],
                reversed: false,
            },
            Coedge {
                edge: second_border,
                reversed: true,
            },
            Coedge {
                edge: cut_edges[0],
                reversed: true,
            },
        ]
    } else {
        vec![
            Coedge {
                edge,
                reversed: true,
            },
            Coedge {
                edge: cut_edges[0],
                reversed: false,
            },
            Coedge {
                edge: second_border,
                reversed: false,
            },
            Coedge {
                edge: cut_edges[1],
                reversed: true,
            },
        ]
    };
    let ring = LoopId(u32::try_from(body.loops.len()).map_err(|_| lost())?);
    body.loops.push(Loop { coedges });
    let normal = surface.derivatives(0.5 * (spans[0].0 + spans[0].1), 0.5);
    let n = normal.1.cross(normal.2);
    let reversed = n.dot(outward) < 0.0;
    let id = FaceId(u32::try_from(body.faces.len()).map_err(|_| lost())?);
    body.faces.push(craft3d_topo::Face {
        surface: Surface::Nurbs(surface),
        reversed,
        loops: vec![ring],
    });
    // The shell of the first face gets it.
    let shell = body
        .shells
        .iter_mut()
        .find(|s| s.faces.contains(&first))
        .ok_or_else(lost)?;
    shell.faces.push(id);
    Ok((id, second_border))
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
    // The vertices of the edges: no two may share one.
    let mut seen = std::collections::BTreeSet::new();
    let mut work = Vec::with_capacity(requests.len());
    for request in requests {
        let data = body
            .edge(request.edge)
            .ok_or(refused(request.edge, Reason::NotAnEdge))?;
        let mut own = vec![data.start];
        if data.end != data.start {
            own.push(data.end);
        }
        for v in own {
            if !seen.insert(v) {
                return Err(refused(request.edge, Reason::FreeMeet));
            }
        }
        let found = super::across(body, &uses, request.edge)?
            .ok_or(refused(request.edge, Reason::NotSolid))?;
        let [a, b] = found.faces;
        let (first, second) = match request.first {
            None => (a, b),
            Some(f) if f == a => (a, b),
            Some(f) if f == b => (b, a),
            Some(f) => {
                return Err(BlendError::Refused {
                    edge: Some(request.edge),
                    face: Some(f),
                    reason: Reason::NotAFace,
                });
            }
        };
        let convex = match found.turn {
            super::Turn::Convex => true,
            super::Turn::Concave => false,
            super::Turn::Smooth => return Err(refused(request.edge, Reason::Smooth)),
        };
        let into = if first == a {
            found.into
        } else {
            [found.into[1], found.into[0]]
        };
        work.push((*request, [first, second], into, convex));
    }
    let mut out = body.clone();
    let mut blends = Vec::with_capacity(requests.len());
    for (index, (request, faces, into, convex)) in work.iter().enumerate() {
        let face = blend_one(&mut out, request, *faces, *into, *convex, tolerance)?;
        blends.push((face, index));
    }
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
    faces.resize(out.faces.len(), Face::Kept(Vec::new()));
    for (face, index) in &blends {
        if let Some(slot) = faces.get_mut(face.0 as usize) {
            *slot = Face::Blend(vec![*index]);
        }
    }
    let replaced: std::collections::BTreeSet<EdgeId> = requests.iter().map(|r| r.edge).collect();
    let edges = (0..out.edges.len())
        .map(|i| {
            let id = EdgeId(u32::try_from(i).unwrap_or(u32::MAX));
            (i < body.edges.len() && !replaced.contains(&id)).then_some(id)
        })
        .collect();
    let vertices = (0..out.vertices.len())
        .map(|i| {
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
