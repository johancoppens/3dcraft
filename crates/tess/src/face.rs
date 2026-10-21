//! One face to triangles, in the parameter space of its surface.
//!
//! 1. **Boundary.** The points of the face's loops (the edge points, see
//!    `edge.rs`) are projected onto the surface and followed around each
//!    loop, keeping the parameters continuous across the seam of a
//!    surface that closes on itself: a point's parameter is the one
//!    nearest its predecessor's, never the raw projection, which may come
//!    back as 0 or as a rounding error below a whole period. The analytic
//!    surfaces say how they close; for a spline surface it is read from
//!    its control net ([`Closure`]).
//! 2. **Region.** The loops become closed chains of segments in the
//!    parameter plane. A loop that winds once around the axis is cut open
//!    along a *seam*, a line from it to the loop on the other side (or to
//!    the end of the surface: a pole or an apex); both sides of the cut
//!    get the same 3D points. Where the other loops leave no room for a
//!    straight seam (holes on every longitude, in rows that overlap), the
//!    seam is made of straight pieces from loop to loop: it enters a hole
//!    at one vertex and goes on from another, and the hole's two
//!    stretches between them become part of the two sides of the cut
//!    ([`Routes`]). Where the parametrisation is singular (a
//!    pole, an apex, a collapsed edge of a patch) a whole stretch of the
//!    parameter boundary is one 3D point; its vertices all carry that
//!    point, and triangles between two of them are dropped at the end.
//!    This follows `craft3d_topo`'s mass properties, which treat winding
//!    loops, poles and apexes the same way.
//! 3. **Triangulation.** A constrained Delaunay triangulation of the
//!    chains (`cdt.rs`), in parameters scaled by the size of the surface's
//!    derivatives so that distances resemble those in space.
//!    Before that, the face says which chords of its edges it cannot do
//!    with (`Boundary::chords_to_refine`): those that cross another part
//!    of the boundary, and those the parameters distort. `body.rs` has
//!    them cut in two and comes back with the new points.
//! 4. **Refinement.** Curved faces are seeded with a grid of interior
//!    points sized from the surface's curvature (for a spline surface
//!    from proven bounds on its second derivatives over the face's part
//!    of it, [`spline_second_bounds`]), and then every triangle is
//!    measured: the surface point at the mean parameters of each free
//!    edge and of the triangle must be within the chordal tolerance of the
//!    edge's midpoint and the triangle's centroid, and the triangle must
//!    face the way the surface does. A triangle that fails gets a point on
//!    its worst edge or at its centroid, until none fails. Boundary edges
//!    are never split: their chords were sized against their own curves.
//!
//! The 3D position of a boundary vertex is always the edge point itself.

use crate::cdt::{Cdt, CdtError, FIRST_VERTEX, Interior, P2, orient};
use crate::edge::arc_segments;
use crate::{TessError, TessParams};
use craft3d_geom::closure::clamped;
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, Face};
use std::f64::consts::FRAC_PI_2;

/// Surface parameters, in the order this face works in (see [`Param`]).
type Uv = [f64; 2];

/// A chord of an edge's polyline: the edge, and the number of the chord
/// counted from the edge's start.
pub(crate) type Chord = (u32, u32);

/// What a face adds to the mesh: its own vertices and triangles over them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FaceMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Where the triangles end: every side of a triangle that lies on the
    /// boundary of the face's region (an edge's chord, a piece of a seam),
    /// from its first point to its second as the triangle, facing
    /// outward, runs. `body.rs` holds this against the edges of the face
    /// (`closes_along_edges`).
    pub border: Vec<(Vec3, Vec3)>,
    /// The sides of triangles that join two points of the boundary across
    /// the face, each once. Two faces may have the same one (see
    /// `body.rs`, `shared_rungs`).
    pub rungs: Vec<(Vec3, Vec3)>,
}

fn fail<T>(face: usize, reason: impl Into<String>) -> Result<T, TessError> {
    Err(TessError::Face {
        face,
        reason: reason.into(),
    })
}

fn unsupported<T>(face: usize, what: impl Into<String>) -> Result<T, TessError> {
    Err(TessError::Unsupported {
        face,
        what: what.into(),
    })
}

/// A surface with its two parameters possibly swapped, so that a loop
/// that winds around the tube of a torus can be handled as one that winds
/// around its axis. Parameter 0 is called `a`, parameter 1 `b`.
#[derive(Clone, Copy)]
struct Param<'a> {
    surface: &'a Surface,
    swapped: bool,
    closure: Closure,
}

/// How a surface closes on itself, in its own `(u, v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Closure {
    /// The period of each parameter that has one.
    periods: [Option<f64>; 2],
    /// For a parameter with a period that the surface is defined on over
    /// one period only (a spline surface): that range. Parameters beyond
    /// it are brought back into it before the surface is evaluated; the
    /// analytic kinds are evaluated wherever they are asked.
    wrap: [Option<(f64, f64)>; 2],
}

/// The part of a spline surface's parameter range below which the extent
/// of a face's boundary is rounding of the projection (see
/// [`face_boundary`]).
const NO_REGION: f64 = 1e-9;

/// How slowly, against where it moves fastest along the loop, a spline
/// surface may move with a parameter at a boundary point for the
/// parameter found there by the search to be taken as it is (see
/// [`build_ring`]).
const NEAR_POLE: f64 = 1e-3;

/// How near, as a part of the size of the control net, two rows of control
/// points must be to count as the same row.
const SAME_ROW: f64 = 1e-9;

/// Whether a net of `count_u` rows of `count_v` control points (row-major)
/// ends, along u and along v, on the row it starts with: the same points,
/// to [`SAME_ROW`] of the net's size, and weights in one proportion, so
/// that the two rows are the same curve. A net all of whose rows are that
/// one row closes nothing: the surface is a curve there.
fn net_closes(count_u: usize, count_v: usize, ctrl: &[Vec3], weights: Option<&[f64]>) -> [bool; 2] {
    if count_u < 2 || count_v < 2 || count_u.checked_mul(count_v) != Some(ctrl.len()) {
        return [false, false];
    }
    let far = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let (mut low, mut high) = (far, -far);
    for p in ctrl {
        (low, high) = (low.min(*p), high.max(*p));
    }
    let near = SAME_ROW * low.distance(high);
    if !(near.is_finite() && near > 0.0) {
        return [false, false];
    }
    let at = |i: usize, j: usize| ctrl.get(i * count_v + j).copied();
    let weight = |i: usize, j: usize| match weights {
        Some(weights) => weights.get(i * count_v + j).copied(),
        None => Some(1.0),
    };
    // Along `rows` rows of `across` points each, with `get(row, k)`.
    let closes = |rows: usize, across: usize, get: &dyn Fn(usize, usize) -> Option<(Vec3, f64)>| {
        let Some(((_, first), (_, last))) = get(0, 0).zip(get(rows - 1, 0)) else {
            return false;
        };
        let mut moves = false;
        for k in 0..across {
            let (Some((p, w)), Some((q, x))) = (get(0, k), get(rows - 1, k)) else {
                return false;
            };
            // `w / first == x / last`, without dividing.
            if exceeds(p.distance(q), near)
                || exceeds((w * last - x * first).abs(), SAME_ROW * (w * last).abs())
            {
                return false;
            }
            for row in 1..rows - 1 {
                moves |= get(row, k).is_some_and(|(r, _)| r.distance(p) > near);
            }
        }
        moves
    };
    [
        closes(count_u, count_v, &|i, j| at(i, j).zip(weight(i, j))),
        closes(count_v, count_u, &|j, i| at(i, j).zip(weight(i, j))),
    ]
}

impl Closure {
    /// The analytic kinds say how they close (`Surface::period_u`). A
    /// spline surface does not: `craft3d-geom` knows no period for it. It
    /// closes in a direction when its control net ends there on the row
    /// it starts with ([`net_closes`]) and its knots are clamped; its
    /// period is then the width of its domain. (A surface that closes
    /// without that, on knots that are not clamped, is not recognised: a
    /// face across its seam is refused, see `mesh_face`.)
    fn of(surface: &Surface) -> Self {
        let ((u0, u1), (v0, v1)) = surface.domain();
        let closes = match surface {
            Surface::Bezier(patch) => net_closes(
                patch.degree_u() + 1,
                patch.degree_v() + 1,
                patch.control_points(),
                None,
            ),
            Surface::Nurbs(s) => {
                let (count_u, count_v) = s.counts();
                let (degree_u, degree_v) = s.degrees();
                let [u, v] = net_closes(count_u, count_v, s.control_points(), Some(s.weights()));
                [
                    u && clamped(s.knots_u(), degree_u),
                    v && clamped(s.knots_v(), degree_v),
                ]
            }
            _ => {
                return Self {
                    periods: [surface.period_u(), surface.period_v()],
                    wrap: [None, None],
                };
            }
        };
        let [closes_u, closes_v] = closes;
        let range = |closes: bool, low: f64, high: f64| {
            (closes && low.is_finite() && high.is_finite() && high > low).then_some((low, high))
        };
        let wrap = [range(closes_u, u0, u1), range(closes_v, v0, v1)];
        Self {
            periods: wrap.map(|range| range.map(|(low, high)| high - low)),
            wrap,
        }
    }
}

/// `x` brought into `range` by whole periods; as it is when it lies in the
/// range already, either end included.
fn wrapped(x: f64, range: Option<(f64, f64)>) -> f64 {
    match range {
        Some((low, high)) if !(x >= low && x <= high) => {
            let back = low + (x - low).rem_euclid(high - low);
            if back.is_finite() { back } else { x }
        }
        _ => x,
    }
}

impl Param<'_> {
    /// `p` in the surface's own order.
    fn raw(&self, p: Uv) -> (f64, f64) {
        let [a, b] = p;
        if self.swapped { (b, a) } else { (a, b) }
    }

    /// Where the surface is evaluated for `p`: [`Param::raw`], brought
    /// back into the domain of a spline surface that closes on itself.
    fn at(&self, p: Uv) -> (f64, f64) {
        let (u, v) = self.raw(p);
        let [wrap_u, wrap_v] = self.closure.wrap;
        (wrapped(u, wrap_u), wrapped(v, wrap_v))
    }

    fn order<T>(&self, u: T, v: T) -> [T; 2] {
        if self.swapped { [v, u] } else { [u, v] }
    }

    fn eval(&self, p: Uv) -> Vec3 {
        let (u, v) = self.at(p);
        self.surface.eval(u, v)
    }

    /// `(point, ∂/∂a, ∂/∂b)`.
    fn derivatives(&self, p: Uv) -> (Vec3, [Vec3; 2]) {
        let (u, v) = self.at(p);
        let (point, du, dv) = self.surface.derivatives(u, v);
        (point, self.order(du, dv))
    }

    /// The parameters of the surface point nearest `point`, which is a
    /// point of an edge of the face and so lies on the surface.
    fn project(&self, point: Vec3) -> Result<Uv, GeomError> {
        let (u, v) = self.surface.project(point)?;
        Ok(self.order(u, v))
    }

    fn periods(&self) -> [Option<f64>; 2] {
        let [u, v] = self.closure.periods;
        self.order(u, v)
    }

    fn domain(&self) -> [(f64, f64); 2] {
        let (u, v) = self.surface.domain();
        self.order(u, v)
    }

    /// The surface's own unit normal `∂u × ∂v`, whatever the order.
    fn normal(&self, p: Uv) -> Option<Vec3> {
        let (u, v) = self.at(p);
        self.surface.normal(u, v)
    }

    /// The normal a vertex of the mesh at `p` is shaded with: the
    /// surface's own, except at a pole of a spline surface (`poles`, found
    /// once per face), where `∂u × ∂v` has no length and its direction is
    /// rounding. There it is the limit normal along the meridian that
    /// leaves the pole at `p`'s parameter along the border: the axis
    /// where the surface closes up smoothly (a revolved profile that meets
    /// the axis at right angles), the normal of that ruling at a cone-like
    /// point.
    fn vertex_normal(&self, p: Uv, poles: &[craft3d_geom::pole::Pole]) -> Option<Vec3> {
        let (u, v) = self.at(p);
        if let Some(pole) = poles.iter().find(|pole| pole.holds(self.surface, u, v)) {
            let along = if pole.across_v() { u } else { v };
            return pole.normal_along(self.surface, along);
        }
        self.surface.normal(u, v)
    }

    /// `∂a × ∂b` points along the surface normal times this.
    fn handedness(&self) -> f64 {
        if self.swapped { -1.0 } else { 1.0 }
    }

    fn is_plane(&self) -> bool {
        matches!(self.surface, Surface::Plane(_))
    }

    fn is_freeform(&self) -> bool {
        matches!(self.surface, Surface::Bezier(_) | Surface::Nurbs(_))
    }
}

/// A boundary vertex of the region.
#[derive(Clone, Copy, Debug)]
struct Bv {
    uv: Uv,
    pos: Vec3,
    /// The parameter that means nothing here: this vertex is on a pole,
    /// an apex or a collapsed edge, where changing it does not move the
    /// point.
    free: Option<usize>,
    /// The chord of an edge that runs from this vertex to the next of its
    /// loop. (Of two vertices on a pole both name the chord that leaves
    /// the second; the stretch between them is told apart later.)
    chord: Option<Chord>,
}

/// One loop in parameter space. Vertex `i` is joined to `i + 1`, and the
/// last to the first moved by `winding` periods.
struct Ring {
    verts: Vec<Bv>,
    winding: [i32; 2],
}

/// `x > 0`; false for NaN.
fn positive(x: f64) -> bool {
    x > 0.0
}

/// Not `a ≤ b`: `a` is larger, or one of them is NaN. Limits are tested
/// with this, so that a NaN is over every limit.
fn exceeds(a: f64, b: f64) -> bool {
    !matches!(
        a.partial_cmp(&b),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

fn get2<T: Copy>(a: [T; 2], k: usize) -> T {
    let [x, y] = a;
    if k == 0 { x } else { y }
}

fn set2<T>(a: &mut [T; 2], k: usize, value: T) {
    let [x, y] = a;
    if k == 0 {
        *x = value;
    } else {
        *y = value;
    }
}

fn tuple(p: Uv) -> P2 {
    let [a, b] = p;
    (a, b)
}

/// `value` moved by whole periods to be as near `to` as it can.
fn nearest(value: f64, to: f64, period: Option<f64>) -> f64 {
    match period {
        Some(p) if p.is_finite() && p > 0.0 => {
            let turns = ((to - value) / p).round();
            if turns.is_finite() {
                value + turns * p
            } else {
                value
            }
        }
        _ => value,
    }
}

fn shifted(uv: Uv, turns: [i32; 2], periods: [Option<f64>; 2]) -> Uv {
    let [a, b] = uv;
    let [ta, tb] = turns;
    let [pa, pb] = periods;
    [
        a + f64::from(ta) * pa.unwrap_or(0.0),
        b + f64::from(tb) * pb.unwrap_or(0.0),
    ]
}

/// The points around one loop, in loop order, without repeating the point
/// where it closes; each with the chord that runs from it to the next.
pub(crate) fn loop_points(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &Face,
    index: usize,
) -> Result<Vec<Vec<(Vec3, Chord)>>, TessError> {
    let mut loops = Vec::with_capacity(face.loops.len());
    for (number, loop_id) in face.loops.iter().enumerate() {
        let loop_ = body.loop_(*loop_id).ok_or_else(|| {
            TessError::InvalidBody(format!("face {index} uses a loop that does not exist"))
        })?;
        let mut points = Vec::new();
        let mut ends = Vec::with_capacity(loop_.coedges.len());
        for coedge in &loop_.coedges {
            let (Some(edge), Some(pair)) =
                (edges.get(coedge.edge.0 as usize), body.coedge_ends(*coedge))
            else {
                return Err(TessError::InvalidBody(format!(
                    "face {index} uses edge {}, which does not exist",
                    coedge.edge.0
                )));
            };
            ends.push(pair);
            // Each coedge contributes all its points but the last, which is
            // the first of the next coedge.
            let count = edge.len().saturating_sub(1);
            let chord = |k: usize| (coedge.edge.0, u32::try_from(k).unwrap_or(u32::MAX));
            if coedge.reversed {
                // Backwards, point `count − j` is followed by the chord
                // that ends there.
                let backwards = edge.iter().rev().take(count).enumerate();
                points.extend(backwards.map(|(j, p)| (*p, chord(count - 1 - j))));
            } else {
                let forwards = edge.iter().take(count).enumerate();
                points.extend(forwards.map(|(j, p)| (*p, chord(j))));
            }
        }
        let closed = ends
            .iter()
            .zip(ends.iter().cycle().skip(1))
            .all(|(this, next)| this.1 == next.0);
        if !closed || points.is_empty() {
            return fail(
                index,
                format!("its loop {number} is not a closed chain of edges"),
            );
        }
        loops.push(points);
    }
    Ok(loops)
}

/// A pole of a spline surface at a corner of its parameters: a border
/// across `a` and a border across `b` that are both the one point. The
/// meridians leave it from both borders, along the L that the two make in
/// the parameters; a boundary that comes into the pole goes along the L
/// from the meridian of the point before to that of the point after
/// ([`CornerPole::foot`]), round the corner if they are on different
/// borders.
#[derive(Clone, Copy, Debug)]
struct CornerPole {
    /// The corner, in the face's order of parameters.
    corner: Uv,
    /// The range of each parameter, signed so that it runs from the corner
    /// into the domain.
    span: [f64; 2],
    /// The two poles.
    poles: [craft3d_geom::pole::Pole; 2],
    /// Points this near the pole are the pole: a billionth of the size of
    /// the surface.
    near: f64,
}

impl CornerPole {
    fn of(param: &Param) -> Option<CornerPole> {
        let found = craft3d_geom::pole::poles(param.surface);
        let [x, y] = found.as_slice() else {
            return None;
        };
        if x.across_v() == y.across_v() {
            return None;
        }
        let ((u0, u1), (v0, v1)) = param.surface.domain();
        let (mut cu, mut cv, mut su, mut sv) = (u0, v0, u1 - u0, v1 - v0);
        for pole in [x, y] {
            match pole.border {
                craft3d_geom::pole::Border::UEnd => (cu, su) = (u1, u0 - u1),
                craft3d_geom::pole::Border::VEnd => (cv, sv) = (v1, v0 - v1),
                _ => {}
            }
        }
        if ![cu, cv, su, sv].iter().all(|x| x.is_finite()) || su == 0.0 || sv == 0.0 {
            return None;
        }
        let size = [(u0, v0), (u0, v1), (u1, v0), (u1, v1)]
            .iter()
            .map(|&(u, v)| param.surface.eval(u, v).distance(x.point))
            .fold(0.0, f64::max);
        Some(CornerPole {
            corner: param.order(cu, cv),
            span: param.order(su, sv),
            poles: [*x, *y],
            near: 1e-9 * size,
        })
    }

    /// The shares of the range from the corner.
    fn shares(&self, uv: Uv) -> [f64; 2] {
        let [a, b] = uv;
        let [ca, cb] = self.corner;
        let [sa, sb] = self.span;
        [(a - ca) / sa, (b - cb) / sb]
    }

    /// Whether the point at `pos`, projected to `uv`, is the pole: `uv`
    /// on the L, or `pos` the pole's point (to rounding: the projection of
    /// the pole may come back a little off the L).
    fn holds(&self, uv: Uv, pos: Vec3) -> bool {
        let [x, y] = self.shares(uv);
        !exceeds(x.abs(), 1e-9)
            || !exceeds(y.abs(), 1e-9)
            || self
                .poles
                .iter()
                .any(|p| !exceeds(p.point.distance(pos), self.near))
    }

    /// Where the meridian through `uv` leaves the L, and the parameter
    /// that is free there. Next to a border of the pole the surface is `P
    /// + s t R(s, t)` with `s` and `t` the shares from the corner, and near
    /// `t = 0` the line of constant `s` leaves the pole in the direction
    /// `R(s, 0)`: it is the meridian of the place `(s, 0)` of the L. So
    /// the foot is `uv` with its smaller share dropped, as at a pole on one
    /// border the free parameter is kept (`build_ring`).
    fn foot(&self, uv: Uv) -> (Uv, usize) {
        let [x, y] = self.shares(uv);
        let [a, b] = uv;
        let [ca, cb] = self.corner;
        if x.abs() <= y.abs() {
            ([ca, b], 1)
        } else {
            ([a, cb], 0)
        }
    }
}

/// Follows one loop through parameter space.
///
/// `sense` is +1 when the loops of this face run counter-clockwise in
/// `(a, b)` and −1 when clockwise.
fn build_ring(
    param: &Param,
    points: &[(Vec3, Chord)],
    sense: f64,
    face: usize,
    number: usize,
) -> Result<Ring, TessError> {
    let periods = param.periods();
    let domain = param.domain();
    let mut raw: Vec<Uv> = Vec::with_capacity(points.len());
    let mut sizes: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    let mut largest = [0.0_f64; 2];
    for &(point, _) in points {
        let uv = param.project(point).map_err(|e| TessError::Face {
            face,
            reason: format!(
                "a point of its loop {number} cannot be projected onto its surface: {e}"
            ),
        })?;
        let [a, b] = uv;
        if !(a.is_finite() && b.is_finite()) {
            return fail(
                face,
                format!("a point of its loop {number} has no finite surface parameters"),
            );
        }
        let (_, [da, db]) = param.derivatives(uv);
        let [la, lb] = &mut largest;
        *la = la.max(da.length());
        *lb = lb.max(db.length());
        raw.push(uv);
        sizes.push([da.length(), db.length()]);
    }
    // On a spline surface with poles a parameter is free only at a pole:
    // on its border, or its point to rounding (a billionth of the
    // surface's size, as at a pole in a corner). Where the surface closes
    // to a higher order (a cusp, whose iso-curves shrink as the square of
    // the distance), the surface moves a billionth as fast as elsewhere a
    // long way short of the pole, and the parameter is still known there:
    // the point lies on its own meridian, not on that of the point before.
    let poles = if param.is_freeform() {
        craft3d_geom::pole::poles(param.surface)
    } else {
        Vec::new()
    };
    let size = {
        let ((u0, u1), (v0, v1)) = param.surface.domain();
        // The largest over all poles: a patch whose corners all lie on one
        // pole is no size from that one.
        poles.iter().fold(0.0, |size: f64, pole| {
            [(u0, v0), (u0, v1), (u1, v0), (u1, v1)]
                .iter()
                .map(|&(u, v)| param.surface.eval(u, v).distance(pole.point))
                .fold(size, f64::max)
        })
    };
    let on_pole: Vec<bool> = raw
        .iter()
        .zip(points)
        .map(|(&uv, &(pos, _))| {
            let (u, v) = param.raw(uv);
            poles.is_empty()
                || poles.iter().any(|pole| {
                    pole.holds(param.surface, u, v)
                        || !exceeds(pole.point.distance(pos), 1e-9 * size)
                })
        })
        .collect();
    // A parameter is free where the surface does not move with it: there
    // the projection returned an arbitrary value for it.
    let free_at = |i: usize| -> Option<usize> {
        if !on_pole.get(i).copied().unwrap_or(true) {
            return None;
        }
        let [sa, sb] = sizes.get(i).copied().unwrap_or([1.0, 1.0]);
        let [la, lb] = largest;
        match (la > 0.0 && sa <= 1e-9 * la, lb > 0.0 && sb <= 1e-9 * lb) {
            (true, false) => Some(0),
            (false, true) => Some(1),
            _ => None,
        }
    };
    let n = points.len();
    // Near a pole of a spline surface, short of it, a parameter is not
    // free but nearly so: the search that projects a point finds it to
    // rounding over how slowly the surface moves with it, which a few
    // millionths of the surface's size from the pole is no digits at all.
    // A boundary whose points zigzag in that parameter bounds triangles
    // that stand inside out, which no refinement cures. Such a point is
    // projected again by descent from its neighbour along the loop, away
    // from the pole, where the parameter is known: from there the descent
    // moves it only as far as the point asks. (Where the analytic kinds
    // have a pole their parameters are in closed form.)
    if param.is_freeform() {
        let near_pole = |i: usize| -> bool {
            let [sa, sb] = sizes.get(i).copied().unwrap_or([1.0, 1.0]);
            let [la, lb] = largest;
            free_at(i).is_none() && (sa <= NEAR_POLE * la || sb <= NEAR_POLE * lb)
        };
        let mut walked = vec![false; n];
        let forwards = (0..n).map(|i| (i, (i + n - 1) % n));
        let backwards = (0..n).rev().map(|i| (i, (i + 1) % n));
        for (i, from) in forwards.chain(backwards) {
            let known = free_at(from).is_none()
                && (!near_pole(from) || walked.get(from).copied().unwrap_or(false));
            if !near_pole(i) || walked.get(i).copied().unwrap_or(true) || !known {
                continue;
            }
            let (Some(&start), Some(&(point, _)), Some(&found)) =
                (raw.get(from), points.get(i), raw.get(i))
            else {
                continue;
            };
            let Ok((u, v)) = param.surface.project_from(point, param.raw(start)) else {
                continue;
            };
            let uv = param.order(u, v);
            // Taken when it is a foot of the point as good as the search's.
            let off = |uv: Uv| param.eval(uv).distance(point);
            if uv.iter().all(|x| x.is_finite())
                && off(uv) <= off(found).max(1e-12 * off(start))
                && let (Some(slot), Some(done)) = (raw.get_mut(i), walked.get_mut(i))
            {
                *slot = uv;
                *done = true;
            }
        }
    }
    let corner = if param.is_freeform() {
        CornerPole::of(param)
    } else {
        None
    };
    let at_corner = |i: usize| match (corner, raw.get(i), points.get(i)) {
        (Some(corner), Some(&uv), Some(&(pos, _))) => corner.holds(uv, pos),
        _ => false,
    };
    let Some(start) = (0..n).find(|&i| free_at(i).is_none() && !at_corner(i)) else {
        return fail(
            face,
            format!("every point of its loop {number} is a singular point of the surface"),
        );
    };
    let at = |i: usize| -> Option<(Uv, Vec3)> { Some((*raw.get(i)?, points.get(i)?.0)) };
    let chord_at = |i: usize| -> Option<Chord> { points.get(i).map(|p| p.1) };
    let Some((first, first_pos)) = at(start) else {
        return fail(face, "an empty loop");
    };
    let mut verts = vec![Bv {
        uv: first,
        pos: first_pos,
        free: None,
        chord: chord_at(start),
    }];
    let mut prev = first;
    for step in 1..n {
        let i = (start + step) % n;
        let Some((cur, pos)) = at(i) else { continue };
        if let Some(corner) = corner.filter(|c| c.holds(cur, pos) || free_at(i).is_some()) {
            // At a pole at a corner of the parameters: arrive on the L
            // along the meridian of the point before, and leave it along
            // that of the point after, round the corner if they are on
            // different borders of it. Points of the loop next to each
            // other at the pole make one stretch of the L.
            let (arrive, free) = corner.foot(prev);
            if !verts.last().is_some_and(|v| v.pos == pos && v.uv == arrive) {
                verts.push(Bv {
                    uv: arrive,
                    pos,
                    free: Some(free),
                    chord: chord_at(i),
                });
            }
            prev = arrive;
            let j = (i + 1) % n;
            let next = at(j).filter(|&(uv, p)| !corner.holds(uv, p) && free_at(j).is_none());
            if let Some((next, _)) = next {
                let (leave, free_out) = corner.foot(next);
                if free_out != free {
                    verts.push(Bv {
                        uv: corner.corner,
                        pos,
                        free: Some(free),
                        chord: chord_at(i),
                    });
                }
                if leave != arrive {
                    verts.push(Bv {
                        uv: leave,
                        pos,
                        free: Some(free_out),
                        chord: chord_at(i),
                    });
                }
                prev = leave;
            }
            continue;
        }
        match free_at(i) {
            None => {
                let uv = [
                    nearest(get2(cur, 0), get2(prev, 0), get2(periods, 0)),
                    nearest(get2(cur, 1), get2(prev, 1), get2(periods, 1)),
                ];
                verts.push(Bv {
                    uv,
                    pos,
                    free: None,
                    chord: chord_at(i),
                });
                prev = uv;
            }
            Some(k) => {
                // Arrive with the free parameter unchanged.
                let m = 1 - k;
                let mut uv = prev;
                set2(
                    &mut uv,
                    m,
                    nearest(get2(cur, m), get2(prev, m), get2(periods, m)),
                );
                verts.push(Bv {
                    uv,
                    pos,
                    free: Some(k),
                    chord: chord_at(i),
                });
                // Leave with the value of the next point. Around a closed
                // parameter there are two ways to get there; the face is
                // to the left of its boundary, which at the upper end of
                // the other parameter means running backwards.
                let j = (i + 1) % n;
                if let (None, Some((next, _))) = (free_at(j), at(j)) {
                    let target = get2(next, k);
                    let here = get2(uv, k);
                    // A loop that leaves the way it came (the seam of a
                    // whole closed surface, given as an edge that the loop
                    // walks to the pole and back) turns all the way
                    // around the pole: the face is everything about it.
                    let back = (i + n - 1) % n;
                    let returns = back != j
                        && points
                            .get(j)
                            .zip(points.get(back))
                            .is_some_and(|(next, last)| next.0 == last.0);
                    let leave = match get2(periods, k) {
                        None => target,
                        Some(p) => {
                            let (low, high) = get2(domain, m);
                            let other = get2(uv, m);
                            let at_high = high.is_finite()
                                && (!low.is_finite() || (other - high).abs() < (other - low).abs());
                            let direction = if (k == 0) == at_high { -sense } else { sense };
                            let delta = (direction * (target - here)).rem_euclid(p);
                            if returns {
                                here + direction * p
                            } else if delta < 1e-9 * p || delta > p * (1.0 - 1e-9) {
                                here
                            } else {
                                here + direction * delta
                            }
                        }
                    };
                    if leave != here && leave.is_finite() {
                        set2(&mut uv, k, leave);
                        verts.push(Bv {
                            uv,
                            pos,
                            free: Some(k),
                            chord: chord_at(i),
                        });
                    }
                }
                prev = uv;
            }
        }
    }
    let mut winding = [0_i32; 2];
    for k in 0..2 {
        if let Some(p) = get2(periods, k) {
            let closing = nearest(get2(first, k), get2(prev, k), Some(p));
            let turns = ((closing - get2(first, k)) / p).round();
            if exceeds(turns.abs(), 1e6) {
                return fail(face, format!("its loop {number} is not finite"));
            }
            // In range by the check above.
            set2(&mut winding, k, turns as i32);
        }
    }
    Ok(Ring { verts, winding })
}

impl Ring {
    /// The far end of the edge that starts at vertex `i`.
    fn next(&self, i: usize, periods: [Option<f64>; 2]) -> Option<Bv> {
        if i + 1 < self.verts.len() {
            self.verts.get(i + 1).copied()
        } else {
            let mut v = *self.verts.first()?;
            v.uv = shifted(v.uv, self.winding, periods);
            Some(v)
        }
    }

    /// Twice the signed area enclosed (meaningful when it does not wind).
    fn twice_area(&self, periods: [Option<f64>; 2]) -> f64 {
        let Some(origin) = self.verts.first().map(|v| v.uv) else {
            return 0.0;
        };
        let [oa, ob] = origin;
        let mut sum = 0.0;
        for (i, v) in self.verts.iter().enumerate() {
            if let Some(w) = self.next(i, periods) {
                let ([a0, b0], [a1, b1]) = (v.uv, w.uv);
                sum += (a0 - oa) * (b1 - ob) - (a1 - oa) * (b0 - ob);
            }
        }
        sum
    }

    /// The near end of the edge that ends at vertex `i`.
    fn prev(&self, i: usize, periods: [Option<f64>; 2]) -> Option<Bv> {
        if i > 0 {
            self.verts.get(i - 1).copied()
        } else {
            let mut v = *self.verts.last()?;
            let [ta, tb] = self.winding;
            v.uv = shifted(v.uv, [-ta, -tb], periods);
            Some(v)
        }
    }

    /// Whether a way that leaves vertex `i` along `towards` goes into the
    /// face: it lies in the corner the face has there, between the edge
    /// that leaves the vertex and the one that comes in, on the face's
    /// side of them (the left when `sense` is positive, as in
    /// [`band_region`]). A loop that passes twice through one place (a
    /// pocket of the face's outside that touches the rest of its
    /// boundary in a point) has two vertices there, and only one of
    /// them opens towards a given side. Along an edge, or where an edge
    /// has no length, it is not ruled out.
    fn opens_towards(&self, i: usize, towards: Uv, sense: f64, periods: [Option<f64>; 2]) -> bool {
        let (Some(here), Some(next), Some(prev)) = (
            self.verts.get(i),
            self.next(i, periods),
            self.prev(i, periods),
        ) else {
            return true;
        };
        let [ha, hb] = here.uv;
        let out = (get2(next.uv, 0) - ha, get2(next.uv, 1) - hb);
        let back = (get2(prev.uv, 0) - ha, get2(prev.uv, 1) - hb);
        let way = tuple(towards);
        // The angle from `x` to `y`, counter-clockwise, in [0, 2π).
        let turn = |x: P2, y: P2| -> Option<f64> {
            let cross = x.0 * y.1 - x.1 * y.0;
            let dot = x.0 * y.0 + x.1 * y.1;
            if !(cross.is_finite() && dot.is_finite()) || (cross == 0.0 && dot == 0.0) {
                return None;
            }
            let angle = cross.atan2(dot);
            Some(if angle < 0.0 {
                angle + std::f64::consts::TAU
            } else {
                angle
            })
        };
        // The corner runs counter-clockwise from `start` to `end`.
        let (start, end) = if positive(sense) {
            (out, back)
        } else {
            (back, out)
        };
        match (turn(start, way), turn(start, end)) {
            (Some(way), Some(corner)) => way == 0.0 || way <= corner,
            _ => true,
        }
    }

    fn mean(&self, k: usize) -> f64 {
        let sum: f64 = self.verts.iter().map(|v| get2(v.uv, k)).sum();
        sum / self.verts.len().max(1) as f64
    }
}

/// A cut through a surface that closes on itself: the straight line in
/// parameter space between two boundary vertices, and its twin one period
/// further, which is the same curve in space. A cut that cannot be
/// straight is several of these, from loop to loop ([`Cut`]).
#[derive(Clone, Copy)]
struct Seam {
    from: usize,
    to: usize,
    twin_from: usize,
    twin_to: usize,
    shift: Uv,
}

/// A loop that does not wind, on the way of a seam (see
/// [`Region::add_split`]): which of the face's loops, moved by how many
/// periods, and the vertices at which the seam arrives and goes on.
#[derive(Clone, Copy, Debug)]
struct Via {
    ring: usize,
    turns: [i32; 2],
    enter: usize,
    leave: usize,
}

/// The vertices of a loop a seam passes through, on the side where the
/// region starts and (`twin_*`) a period further.
struct SplitEnds {
    enter: usize,
    leave: usize,
    twin_enter: usize,
    twin_leave: usize,
}

/// Where a face that goes around its surface is cut open: from the lower
/// winding loop (or the lower end of the surface) to the upper one, in
/// one straight piece or, when the other loops leave no room for that,
/// in pieces from loop to loop.
struct Cut {
    /// Where the seam starts on the lower end of the surface, when no
    /// loop winds there.
    a: f64,
    /// The same for the upper end.
    upper_a: f64,
    lower_start: Option<usize>,
    upper_start: Option<(usize, i32)>,
    /// The loops on the way, from the lower side to the upper.
    via: Vec<Via>,
}

/// The boundary of a face in parameter space.
#[derive(Default)]
struct Region {
    verts: Vec<Bv>,
    segs: Vec<(usize, usize)>,
    /// Per segment, the chord of an edge it is, if any: seams and the
    /// stretches of a pole are not.
    chords: Vec<Option<Chord>>,
    seams: Vec<Seam>,
    /// What is wrong with the region as it stands, when that may be the
    /// doing of chords that are too coarse: a hole whose first point lies
    /// outside the polygon of the loop around it. The face is refused
    /// with this error unless finer chords make it go away.
    doubt: Option<TessError>,
}

impl Region {
    fn add(&mut self, v: Bv) -> usize {
        self.verts.push(v);
        self.verts.len() - 1
    }

    /// A segment from vertex `i` to vertex `j`.
    fn link(&mut self, i: usize, j: usize, chord: Option<Chord>) {
        self.segs.push((i, j));
        self.chords.push(chord);
    }

    /// A loop that does not wind, moved by whole periods.
    fn add_ring(&mut self, ring: &Ring, turns: [i32; 2], periods: [Option<f64>; 2]) {
        let base = self.verts.len();
        let n = ring.verts.len();
        for v in &ring.verts {
            self.verts.push(Bv {
                uv: shifted(v.uv, turns, periods),
                ..*v
            });
        }
        for (i, v) in ring.verts.iter().enumerate() {
            self.link(base + i, base + (i + 1) % n, v.chord);
        }
    }

    /// A loop that winds around parameter `a`, cut open at vertex `start`
    /// and moved by `turns` periods. Returns its ends: the vertex with the
    /// lower `a` first.
    fn add_chain(
        &mut self,
        ring: &Ring,
        start: usize,
        turns: [i32; 2],
        periods: [Option<f64>; 2],
    ) -> (usize, usize) {
        let base = self.verts.len();
        let n = ring.verts.len();
        let [wa, _] = ring.winding;
        let mut leaving: Option<Chord> = None;
        for i in 0..=n {
            let Some(v) = ring.verts.get((start + i) % n.max(1)) else {
                continue;
            };
            let [ta, tb] = turns;
            let wrapped = if start + i >= n { wa } else { 0 };
            self.verts.push(Bv {
                uv: shifted(v.uv, [ta + wrapped, tb], periods),
                ..*v
            });
            if i > 0 {
                self.link(base + i - 1, base + i, leaving);
            }
            leaving = v.chord;
        }
        if wa >= 0 {
            (base, base + n)
        } else {
            (base + n, base)
        }
    }

    /// A loop that does not wind and that a seam passes through: it comes
    /// in at vertex `enter` and goes on from vertex `leave`. The loop is
    /// then part of both sides of the cut. Of its two stretches between
    /// those vertices one is added at `turns` and bounds the region where
    /// it starts, the other a period of `a` further, where the region
    /// ends; `forward_far` says that the far one is the stretch that runs
    /// from `enter` to `leave` in loop order.
    ///
    /// Returns the vertices the seam's pieces end on.
    fn add_split(
        &mut self,
        ring: &Ring,
        via: &Via,
        forward_far: bool,
        periods: [Option<f64>; 2],
    ) -> SplitEnds {
        let n = ring.verts.len();
        let [ta, tb] = via.turns;
        let far = [ta.saturating_add(1), tb];
        if via.enter == via.leave || n == 0 {
            // The seam only touches the loop: all of it stays on the near
            // side, and the far side gets the one point.
            let base = self.verts.len();
            self.add_ring(ring, via.turns, periods);
            let near = base + via.enter.min(n.saturating_sub(1));
            let twin = match ring.verts.get(via.enter) {
                Some(v) => self.add(Bv {
                    uv: shifted(v.uv, far, periods),
                    chord: None,
                    ..*v
                }),
                None => near,
            };
            return SplitEnds {
                enter: near,
                leave: near,
                twin_enter: twin,
                twin_leave: twin,
            };
        }
        // The stretch from vertex `from` on to vertex `to`: its first and
        // its last vertex.
        let mut stretch = |from: usize, to: usize, turns: [i32; 2]| -> (usize, usize) {
            let base = self.verts.len();
            let steps = (to + n - from) % n;
            let mut leaving: Option<Chord> = None;
            for i in 0..=steps {
                let Some(v) = ring.verts.get((from + i) % n) else {
                    continue;
                };
                self.verts.push(Bv {
                    uv: shifted(v.uv, turns, periods),
                    ..*v
                });
                if i > 0 {
                    self.link(base + i - 1, base + i, leaving);
                }
                leaving = v.chord;
            }
            (base, base + steps)
        };
        let (forward_turns, backward_turns) = if forward_far {
            (far, via.turns)
        } else {
            (via.turns, far)
        };
        let (forward_enter, forward_leave) = stretch(via.enter, via.leave, forward_turns);
        let (backward_leave, backward_enter) = stretch(via.leave, via.enter, backward_turns);
        if forward_far {
            SplitEnds {
                enter: backward_enter,
                leave: backward_leave,
                twin_enter: forward_enter,
                twin_leave: forward_leave,
            }
        } else {
            SplitEnds {
                enter: forward_enter,
                leave: forward_leave,
                twin_enter: backward_enter,
                twin_leave: backward_leave,
            }
        }
    }

    fn uv(&self, i: usize) -> Option<Uv> {
        self.verts.get(i).map(|v| v.uv)
    }

    /// Whether `p` is enclosed by the segments and seams (even–odd).
    fn encloses(&self, p: Uv) -> bool {
        let [pa, pb] = p;
        let seam_ends = self
            .seams
            .iter()
            .flat_map(|s| [(s.from, s.to), (s.twin_from, s.twin_to)]);
        let mut inside = false;
        for (i, j) in self.segs.iter().copied().chain(seam_ends) {
            let (Some([a0, b0]), Some([a1, b1])) = (self.uv(i), self.uv(j)) else {
                continue;
            };
            if (b0 > pb) != (b1 > pb) {
                let a = a0 + (pb - b0) / (b1 - b0) * (a1 - a0);
                if a > pa {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// True when the seam from `a` to `b` cannot be used next to the
/// boundary segment (c, d): they cross, or one passes through an end of
/// the other. Meeting end to end is fine: a seam starts and ends on
/// boundary vertices. (Also what two segments of the boundary may not do
/// to each other, see `Boundary::chords_to_refine`.)
fn seam_blocked(a: Uv, b: Uv, c: Uv, d: Uv) -> bool {
    let (ta, tb, tc, td) = (tuple(a), tuple(b), tuple(c), tuple(d));
    let (side_c, side_d) = (orient(ta, tb, tc), orient(ta, tb, td));
    let (side_a, side_b) = (orient(tc, td, ta), orient(tc, td, tb));
    if side_c * side_d < 0.0 && side_a * side_b < 0.0 {
        return true;
    }
    // `p` on the line through (q, r), strictly between them.
    let within = |p: P2, q: P2, r: P2| {
        let along = (p.0 - q.0) * (r.0 - q.0) + (p.1 - q.1) * (r.1 - q.1);
        let whole = (r.0 - q.0) * (r.0 - q.0) + (r.1 - q.1) * (r.1 - q.1);
        p != q && p != r && along > 0.0 && along < whole
    };
    (side_c == 0.0 && within(tc, ta, tb))
        || (side_d == 0.0 && within(td, ta, tb))
        || (side_a == 0.0 && within(ta, tc, td))
        || (side_b == 0.0 && within(tb, tc, td))
}

/// True when an end of the boundary segment (c, d) that is not an end of
/// the seam from `a` to `b` lies on the seam but for rounding, strictly
/// between its ends, or an end of the seam on the segment. A seam along
/// a straight edge of the loop it starts on (a line of the surface, as a
/// bore with a stepped bottom has at `a = ±π`) runs through the vertex
/// where that edge ends; computed a period further, the seam can lean a
/// last bit away from it, so that [`seam_blocked`]'s exact test sees
/// neither a crossing nor a touch, and the seam swallowed the edge: the
/// face's mesh had no side on its chord (the engine block's bored
/// mounts, 2026-10-15).
fn seam_grazes(a: Uv, b: Uv, c: Uv, d: Uv, period: f64) -> bool {
    const ROUNDING: f64 = 1e-12;
    // Rounding per parameter: of the angle around (and its period), and
    // of the other parameter, which may be a length of any size. Each is
    // measured in its own rounding, so that the test is the same for a
    // body at any scale.
    let reach = |k: usize, start: f64| {
        [a, b, c, d]
            .iter()
            .fold(start, |m, p| m.max(get2(*p, k).abs()))
    };
    let (slack_a, slack_b) = (ROUNDING * reach(0, period.abs()), ROUNDING * reach(1, 0.0));
    if !(slack_a > 0.0 && slack_b > 0.0) {
        return false;
    }
    let norm = |p: Uv| -> P2 {
        let [x, y] = p;
        (x / slack_a, y / slack_b)
    };
    let (a, b, c, d) = (norm(a), norm(b), norm(c), norm(d));
    // `p` within one rounding of the segment (q, r), strictly between its
    // ends and not at either of them.
    let on = |p: P2, q: P2, r: P2| -> bool {
        let (dx, dy) = (r.0 - q.0, r.1 - q.1);
        let whole = dx * dx + dy * dy;
        if !(whole > 0.0 && whole.is_finite()) {
            return false;
        }
        let (px, py) = (p.0 - q.0, p.1 - q.1);
        let along = px * dx + py * dy;
        let away = (px * dy - py * dx).abs() / whole.sqrt();
        let near_end = |e: P2| (p.0 - e.0).abs() <= 1.0 && (p.1 - e.1).abs() <= 1.0;
        away <= 1.0 && along > 0.0 && along < whole && !near_end(q) && !near_end(r)
    };
    on(c, a, b) || on(d, a, b) || on(a, c, d) || on(b, c, d)
}

/// `p`, or the end of a seam it is but for rounding.
///
/// The end of a seam on a loop is that loop's vertex moved by whole
/// periods, computed the way the seam needs it (the upper loop of a band
/// that runs clockwise: a period back, then so many on); the loop's own
/// segments that end there are moved by whole periods at once. The two
/// can differ in the last bit, and the segments then cross the seam they
/// only touch: every straight seam to such a vertex was refused. (That
/// is how the wall of a shank with a long thread came to have no seam
/// at fine tolerances, and to ask for finer chords round after round.)
fn snapped(p: Uv, ends: [Uv; 2], period: f64) -> Uv {
    const ROUNDING: f64 = 1e-13;
    let same = |x: Uv, y: Uv| {
        let ([xa, xb], [ya, yb]) = (x, y);
        (xa - ya).abs() <= ROUNDING * (xa.abs() + ya.abs() + period.abs())
            && (xb - yb).abs() <= ROUNDING * (xb.abs() + yb.abs())
    };
    ends.into_iter().find(|&end| same(p, end)).unwrap_or(p)
}

/// Moves a loop that does not wind by whole periods until `probe` says
/// its first proper vertex is where it belongs.
fn place(ring: &Ring, periods: [Option<f64>; 2], probe: impl Fn(Uv) -> bool) -> Option<[i32; 2]> {
    let vertex = ring
        .verts
        .iter()
        .find(|v| v.free.is_none())
        .or(ring.verts.first())?;
    place_by(vertex, periods, probe)
}

/// A hole that [`place`] finds no place for: where it must be if any of
/// its vertices is where it belongs (the others are then outside the
/// polygon around it, which chords too coarse can do), and where it was
/// found otherwise.
fn place_in_doubt(ring: &Ring, periods: [Option<f64>; 2], probe: impl Fn(Uv) -> bool) -> [i32; 2] {
    ring.verts
        .iter()
        .filter(|v| v.free.is_none())
        .find_map(|v| place_by(v, periods, &probe))
        .unwrap_or([0, 0])
}

fn place_by(
    vertex: &Bv,
    periods: [Option<f64>; 2],
    probe: impl Fn(Uv) -> bool,
) -> Option<[i32; 2]> {
    let range = |period: Option<f64>| if period.is_some() { -2..=2 } else { 0..=0 };
    // Nearest first, so that a loop already in place stays there.
    let mut candidates: Vec<[i32; 2]> = Vec::new();
    for ta in range(get2(periods, 0)) {
        for tb in range(get2(periods, 1)) {
            candidates.push([ta, tb]);
        }
    }
    candidates.sort_by_key(|[ta, tb]| (ta.abs() + tb.abs(), *ta, *tb));
    candidates
        .into_iter()
        .find(|turns| probe(shifted(vertex.uv, *turns, periods)))
}

/// The region of a face whose loops do not wind: an outer loop and holes.
fn patch_region(
    param: &Param,
    rings: &[Ring],
    sense: f64,
    face: usize,
    starved: &mut bool,
) -> Result<Region, TessError> {
    let periods = param.periods();
    let mut region = Region::default();
    let areas: Vec<f64> = rings.iter().map(|r| r.twice_area(periods)).collect();
    let total: f64 = areas.iter().sum();
    if !positive(total * sense) {
        *starved = true;
        return fail(
            face,
            "its loops do not bound a region: they enclose no area, or run the wrong way \
             round (the face would be everything outside them)",
        );
    }
    let outer = areas
        .iter()
        .enumerate()
        .max_by(|x, y| x.1.abs().total_cmp(&y.1.abs()).then(y.0.cmp(&x.0)))
        .map_or(0, |(i, _)| i);
    if let Some(ring) = rings.get(outer) {
        region.add_ring(ring, [0, 0], periods);
    }
    // Every other loop is a hole in the outer one. On a closed surface
    // the projection may have put it a whole period away.
    let mut outline = Region::default();
    outline.verts.clone_from(&region.verts);
    outline.segs.clone_from(&region.segs);
    outline.chords.clone_from(&region.chords);
    for (i, ring) in rings.iter().enumerate() {
        if i == outer {
            continue;
        }
        let turns = place(ring, periods, |p| outline.encloses(p)).unwrap_or_else(|| {
            region.doubt.get_or_insert(TessError::Face {
                face,
                reason: format!(
                    "its loops do not bound a region: loop {i} lies outside the loop that \
                     should contain it"
                ),
            });
            place_in_doubt(ring, periods, |p| outline.encloses(p))
        });
        region.add_ring(ring, turns, periods);
    }
    Ok(region)
}

/// The point a whole end of the parameter range collapses to, when it
/// does: a pole of a sphere, the apex of a cone.
fn pole(param: &Param, a: f64, b: f64, typical: f64, face: usize) -> Result<Bv, TessError> {
    if !b.is_finite() {
        return fail(
            face,
            "it is not bounded: a loop winds around the axis and the surface has no end on \
             that side",
        );
    }
    let (pos, [da, _]) = param.derivatives([a, b]);
    if !(pos.is_finite() && da.length() <= 1e-9 * typical) {
        return fail(
            face,
            "it is not bounded: a loop winds around the axis and the surface does not close \
             to a point on that side",
        );
    }
    Ok(Bv {
        uv: [a, b],
        pos,
        free: Some(0),
        chord: None,
    })
}

/// What a face is told when its triangulation (or the one the search for
/// its seam lays out) runs out of its work budget (`Cdt::spend`).
const SLOW_TRIANGULATION: &str = "its triangulation did not converge: placing its points took \
                                  more work than their number justifies (the boundary is too \
                                  fine or too tangled in parameter space); use a larger \
                                  tolerance";

/// How many places are tried for the seam of a face that goes around its
/// surface.
const SEAM_TRIES: usize = 16;

/// Whether the surface closes all the way around parameter `a` and to a
/// point at both ends of `b`, as a sphere does. It is then a closed
/// surface, on which loops that run clockwise bound a face as well:
/// everything outside them.
fn closes_to_points(param: &Param) -> bool {
    let [Some(_), None] = param.periods() else {
        return false;
    };
    let [(a_low, _), (b_low, b_high)] = param.domain();
    if !(a_low.is_finite() && b_low.is_finite() && b_high.is_finite()) {
        return false;
    }
    let speed = |b: f64| {
        let (_, [da, _]) = param.derivatives([a_low, b]);
        da.length()
    };
    let typical = speed(0.5 * (b_low + b_high));
    positive(typical)
        && typical.is_finite()
        && speed(b_low) <= 1e-9 * typical
        && speed(b_high) <= 1e-9 * typical
}

/// The region of a face that goes all the way around parameter `a`:
/// between two loops that wind around it, or between one and the end of
/// the surface, or (without loops that wind) the whole surface; less the
/// holes its other loops bound.
fn band_region(
    param: &Param,
    rings: &[Ring],
    sense: f64,
    face: usize,
    starved: &mut bool,
) -> Result<Region, TessError> {
    let periods = param.periods();
    let [Some(period), period_b] = periods else {
        return fail(face, "a face without loops on a surface that is not closed");
    };
    let [(a_low, _), (b_low, b_high)] = param.domain();
    // How fast the surface moves with `a` where it is not singular.
    let mut typical = 0.0_f64;

    // The face lies to the left of its loops: above (towards +b) one that
    // runs counter-clockwise around the axis, below one that runs
    // clockwise.
    let mut lower: Option<&Ring> = None;
    let mut upper: Option<&Ring> = None;
    let mut holes: Vec<(usize, &Ring)> = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        let [wa, _] = ring.winding;
        let slot = match f64::from(wa) * sense {
            t if t > 0.0 => &mut lower,
            t if t < 0.0 => &mut upper,
            _ => {
                holes.push((i, ring));
                continue;
            }
        };
        if slot.is_some() {
            return fail(
                face,
                "its loops do not bound a region: two of them wind around the axis the same \
                 way",
            );
        }
        *slot = Some(ring);
    }
    for ring in lower.iter().chain(upper.iter()) {
        for v in &ring.verts {
            let (_, [da, _]) = param.derivatives(v.uv);
            typical = typical.max(da.length());
        }
    }
    if lower.is_none() && upper.is_none() && b_low.is_finite() && b_high.is_finite() {
        let (_, [da, _]) = param.derivatives([a_low, 0.5 * (b_low + b_high)]);
        typical = da.length();
    }
    if !(typical > 0.0 && typical.is_finite()) {
        typical = 1.0;
    }

    // The upper loop, moved up by whole periods of `b` to lie above the
    // lower one (only a torus has such a period).
    let mut upper_turns_b = 0_i32;
    if let (Some(low), Some(up)) = (lower, upper) {
        let gap = up.mean(1) - low.mean(1);
        match period_b {
            Some(pb) => {
                let turns = (-gap / pb).floor() + 1.0;
                if exceeds(turns.abs(), 1e6) {
                    return fail(face, "its loops are not finite");
                }
                // In range by the check above.
                upper_turns_b = if gap > 0.0 { 0 } else { turns as i32 };
            }
            None => {
                if !positive(gap) {
                    return fail(
                        face,
                        "its loops do not bound a region: the face would lie outside both \
                         of the loops that wind around the axis",
                    );
                }
            }
        }
    }
    if lower.is_none() && upper.is_none() && period_b.is_some() {
        return torus_region(param, period, period_b.unwrap_or(period));
    }

    // The seam: from a vertex of one winding loop to the vertex of the
    // other that is nearest in `a`, or straight to the end of the surface.
    // It must not cross any loop; a few starting vertices are tried.
    let left_a = |ring: &Ring, s: usize| -> Option<f64> {
        let [a, _] = ring.verts.get(s)?.uv;
        let [wa, _] = ring.winding;
        Some(if wa < 0 { a - period } else { a })
    };
    // The vertex of `ring` nearest in `a` among those `opens` lets the
    // seam end at (given the vertex and its turns).
    let nearest_vertex =
        |ring: &Ring, a: f64, opens: &dyn Fn(usize, i32) -> bool| -> Option<(usize, i32)> {
            let mut best: Option<(f64, usize, i32)> = None;
            for (j, v) in ring.verts.iter().enumerate() {
                if v.free.is_some() {
                    continue;
                }
                let natural = left_a(ring, j)?;
                let turns = ((a - natural) / period).round();
                let distance = (natural + turns * period - a).abs();
                if turns.abs() <= 1e6
                && best.is_none_or(|(d, _, _)| distance < d)
                // In range by the check before.
                && opens(j, turns as i32)
                {
                    // In range by the check above.
                    best = Some((distance, j, turns as i32));
                }
            }
            best.map(|(_, j, turns)| (j, turns))
        };
    let crosses_a_loop = |from: Uv, to: Uv| -> bool {
        for ring in rings {
            for (i, v) in ring.verts.iter().enumerate() {
                let Some(w) = ring.next(i, periods) else {
                    continue;
                };
                let around = ((get2(from, 0) - get2(v.uv, 0)) / period).round();
                if exceeds(around.abs(), 1e6) {
                    return true;
                }
                // In range by the check above.
                let around = around as i32;
                let b_turns = if period_b.is_some() { -2..=2 } else { 0..=0 };
                for tb in b_turns {
                    for ta in around - 1..=around + 1 {
                        let p = snapped(shifted(v.uv, [ta, tb], periods), [from, to], period);
                        let q = snapped(shifted(w.uv, [ta, tb], periods), [from, to], period);
                        if seam_blocked(from, to, p, q) || seam_grazes(from, to, p, q, period) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    };
    // Where a winding loop is missing, the surface must end in a point.
    if lower.is_none() {
        pole(param, a_low, b_low, typical, face)?;
    }
    if upper.is_none() {
        pole(param, a_low, b_high, typical, face)?;
    }
    // A straight seam from vertex `s` of the loop it starts on (the lower
    // winding loop, or the upper one when there is no lower), when nothing
    // lies across it.
    let straight_from = |anchor: &Ring, s: usize| -> Option<Cut> {
        let v = anchor.verts.get(s).filter(|v| v.free.is_none())?;
        let a = left_a(anchor, s)?;
        let [_, b] = v.uv;
        let (candidate, [from, to]) = if lower.is_some() {
            match upper {
                Some(up) => {
                    // The seam goes up from the lower loop into the face,
                    // and comes down from the upper one into it.
                    let opens = |j: usize, turns: i32| -> bool {
                        let (Some(end_a), Some(end)) = (left_a(up, j), up.verts.get(j)) else {
                            return false;
                        };
                        let [_, end_b] = shifted(end.uv, [0, upper_turns_b], periods);
                        let end_a = end_a + f64::from(turns) * period;
                        let way = [end_a - a, end_b - b];
                        anchor.opens_towards(s, way, sense, periods)
                            && up.opens_towards(j, [-way[0], -way[1]], sense, periods)
                    };
                    let (j, turns) = nearest_vertex(up, a, &opens)?;
                    let (end_a, end) = (left_a(up, j)?, up.verts.get(j)?);
                    let [_, end_b] = shifted(end.uv, [0, upper_turns_b], periods);
                    let end = [end_a + f64::from(turns) * period, end_b];
                    (
                        Cut {
                            a,
                            upper_a: a,
                            lower_start: Some(s),
                            upper_start: Some((j, turns)),
                            via: Vec::new(),
                        },
                        [[a, b], end],
                    )
                }
                None => (
                    Cut {
                        a,
                        upper_a: a,
                        lower_start: Some(s),
                        upper_start: None,
                        via: Vec::new(),
                    },
                    [[a, b], [a, b_high]],
                ),
            }
        } else {
            (
                Cut {
                    a,
                    upper_a: a,
                    lower_start: None,
                    upper_start: Some((s, 0)),
                    via: Vec::new(),
                },
                [[a, b_low], [a, b]],
            )
        };
        // A seam to a pole leaves its loop into the face: up from a lower
        // loop, down from an upper one (between two loops, the choice of
        // the upper vertex has seen to both ends).
        let opens = match (lower.is_some(), upper.is_some()) {
            (true, true) => true,
            (true, false) => anchor.opens_towards(s, [0.0, 1.0], sense, periods),
            (false, _) => anchor.opens_towards(s, [0.0, -1.0], sense, periods),
        };
        (opens && from.iter().chain(&to).all(|x| x.is_finite()) && !crosses_a_loop(from, to))
            .then_some(candidate)
    };
    let anchor = lower.or(upper);
    let mut cut: Option<Cut> = None;
    match anchor {
        None => {
            // From one end of the surface to the other, along the first
            // of a few meridians that no hole lies on. Without holes that
            // is the surface's own seam.
            let tries = if holes.is_empty() { 1 } else { SEAM_TRIES };
            for c in 0..tries {
                let a = a_low + period * c as f64 / tries as f64;
                let (from, to) = ([a, b_low], [a, b_high]);
                if from.iter().chain(&to).all(|x| x.is_finite()) && !crosses_a_loop(from, to) {
                    cut = Some(Cut {
                        a,
                        upper_a: a,
                        lower_start: None,
                        upper_start: None,
                        via: Vec::new(),
                    });
                    break;
                }
            }
        }
        Some(anchor) => {
            let n = anchor.verts.len();
            let tries = n.min(SEAM_TRIES);
            cut = (0..tries)
                .map(|c| c * n / tries)
                .find_map(|s| straight_from(anchor, s));
        }
    }
    // No straight seam: one in pieces, from loop to loop.
    if cut.is_none() {
        let search = Routes {
            param,
            lower,
            upper,
            holes: &holes,
            upper_turns_b,
            crosses_a_loop: &crosses_a_loop,
        };
        cut = search.cut().map_err(|error| TessError::Face {
            face,
            reason: error.into(),
        })?;
    }
    // Nor one in pieces: a straight seam from the far side of the loop it
    // starts on. A loop can run round the axis many times on its way (the
    // land of a thread winds up and back down, one helical edge after the
    // other); the vertices spread along it then all lie below other turns
    // of it, and the search in pieces, which lays the loops out only a
    // turn either way, finds no way past them either. Nothing of the loop
    // lies across the way up from its highest vertex (down from the
    // lowest of an upper loop), and the surface has no hole there.
    if cut.is_none()
        && let Some(anchor) = anchor
    {
        let mut extreme: Vec<usize> = (0..anchor.verts.len())
            .filter(|&s| anchor.verts.get(s).is_some_and(|v| v.free.is_none()))
            .collect();
        let height = |s: &usize| anchor.verts.get(*s).map_or(0.0, |v| get2(v.uv, 1));
        extreme.sort_by(|x, y| {
            let order = height(x).total_cmp(&height(y)).then(x.cmp(y));
            if lower.is_some() {
                order.reverse()
            } else {
                order
            }
        });
        cut = extreme
            .into_iter()
            .take(SEAM_TRIES)
            .find_map(|s| straight_from(anchor, s));
    }
    let Some(cut) = cut else {
        // Chords too coarse may be what closes the way (two loops nearer
        // to each other than the tolerance): the face asks for finer ones
        // before this error stands.
        *starved = true;
        return unsupported(
            face,
            "a face around the axis of its surface with no way for a seam between its loops",
        );
    };

    let mut region = Region::default();
    let (lower_left, lower_right) = match (lower, cut.lower_start) {
        (Some(ring), Some(start)) => region.add_chain(ring, start, [0, 0], periods),
        _ => {
            let left = pole(param, cut.a, b_low, typical, face)?;
            let right = Bv {
                uv: [cut.a + period, b_low],
                ..left
            };
            let ends = (region.add(left), region.add(right));
            region.link(ends.0, ends.1, None);
            ends
        }
    };
    let (upper_left, upper_right) = match (upper, cut.upper_start) {
        (Some(ring), Some((start, turns))) => {
            region.add_chain(ring, start, [turns, upper_turns_b], periods)
        }
        _ => {
            let left = pole(param, cut.upper_a, b_high, typical, face)?;
            let right = Bv {
                uv: [cut.upper_a + period, b_high],
                ..left
            };
            let ends = (region.add(left), region.add(right));
            region.link(ends.0, ends.1, None);
            ends
        }
    };
    // The seam's pieces, through the loops on its way. Going up, the face
    // is on the right of the seam where the region starts and on the left
    // where it ends, and the face is on the left of its loops, which is
    // the right in `(a, b)` when they run clockwise there.
    let (mut from, mut twin_from) = (lower_left, lower_right);
    for via in &cut.via {
        let Some(ring) = rings.get(via.ring) else {
            continue;
        };
        let ends = region.add_split(ring, via, positive(sense), periods);
        region.seams.push(Seam {
            from,
            to: ends.enter,
            twin_from,
            twin_to: ends.twin_enter,
            shift: [period, 0.0],
        });
        (from, twin_from) = (ends.leave, ends.twin_leave);
    }
    region.seams.push(Seam {
        from,
        to: upper_left,
        twin_from,
        twin_to: upper_right,
        shift: [period, 0.0],
    });
    holes.retain(|(i, _)| cut.via.iter().all(|via| via.ring != *i));
    let mut outline = Region::default();
    outline.verts.clone_from(&region.verts);
    outline.segs.clone_from(&region.segs);
    outline.chords.clone_from(&region.chords);
    outline.seams.clone_from(&region.seams);
    for (i, ring) in holes {
        let turns = place(ring, periods, |p| outline.encloses(p)).unwrap_or_else(|| {
            region.doubt.get_or_insert(TessError::Face {
                face,
                reason: format!(
                    "its loops do not bound a region: loop {i} lies outside the region its \
                     other loops bound"
                ),
            });
            place_in_doubt(ring, periods, |p| outline.encloses(p))
        });
        region.add_ring(ring, turns, periods);
    }
    Ok(region)
}

/// How many copies of the loops, side by side around the axis, the search
/// for a seam in pieces lays out on either side of the one in the middle.
const ROUTE_TURNS: i32 = 1;

/// Whether `p` lies above (towards `+b` of) a loop that winds around `a`
/// and is moved by `tb` periods of `b`.
fn above(ring: &Ring, tb: i32, p: Uv, periods: [Option<f64>; 2]) -> bool {
    let [Some(period), _] = periods else {
        return false;
    };
    let [pa, pb] = p;
    let mut odd = false;
    for (i, v) in ring.verts.iter().enumerate() {
        let Some(w) = ring.next(i, periods) else {
            continue;
        };
        let around = ((pa - get2(v.uv, 0)) / period).round();
        if exceeds(around.abs(), 1e6) {
            return false;
        }
        // In range by the check above.
        let around = around as i32;
        for ta in around - 1..=around + 1 {
            let [a0, b0] = shifted(v.uv, [ta, tb], periods);
            let [a1, b1] = shifted(w.uv, [ta, tb], periods);
            if (a0 > pa) != (a1 > pa) && b0 + (pa - a0) / (a1 - a0) * (b1 - b0) < pb {
                odd = !odd;
            }
        }
    }
    odd
}

/// One loop a seam in pieces may start from, pass through or end on.
struct Station<'a> {
    ring: &'a Ring,
    /// Its number among the face's loops, for one that does not wind.
    hole: Option<usize>,
    /// The periods of `b` it is moved by.
    tb: i32,
}

/// A piece a seam may be made of: from vertex `leave` of station `from`
/// to vertex `enter` of station `to`, which lies `dt` periods of `a`
/// further than `from` does. A piece without one of the vertices starts
/// or ends on the end of the surface, straight below or above its other
/// end.
#[derive(Clone, Copy)]
struct Bridge {
    from: usize,
    to: usize,
    leave: Option<usize>,
    enter: Option<usize>,
    dt: i32,
    length: f64,
}

/// How a station was reached from the lower side.
#[derive(Clone, Copy)]
struct Reached {
    /// The bridge it was reached by; none for the station the search
    /// starts from.
    by: Option<usize>,
    /// The periods of `a` the station is moved by.
    turns: i32,
    piece: Option<[Uv; 2]>,
}

/// The search for a seam in pieces: from the lower winding loop (or the
/// lower end of the surface) to the upper one, each piece a straight
/// line between vertices of two loops that crosses nothing.
///
/// The loops are laid out a few times side by side and triangulated,
/// their segments as constraints. No edge of that triangulation crosses
/// a loop, so every edge between two different loops is a piece a seam
/// can use, and because the region between the loops is connected the
/// edges join the lower side to the upper. The shortest chain of them (in
/// pieces, then in length) is the seam. Every piece is still tested
/// against all loops and against the pieces before it, since the layout
/// is finite and a chain is put together from edges found in different
/// copies.
struct Routes<'a> {
    param: &'a Param<'a>,
    lower: Option<&'a Ring>,
    upper: Option<&'a Ring>,
    holes: &'a [(usize, &'a Ring)],
    upper_turns_b: i32,
    crosses_a_loop: &'a dyn Fn(Uv, Uv) -> bool,
}

impl Routes<'_> {
    /// The periods of `b` that bring a loop that does not wind between
    /// the winding loops (only a torus has such a period).
    fn hole_turns(&self, ring: &Ring, periods: [Option<f64>; 2]) -> Option<i32> {
        if get2(periods, 1).is_none() {
            return Some(0);
        }
        let vertex = ring.verts.iter().find(|v| v.free.is_none())?;
        [0, -1, 1, -2, 2].into_iter().find(|&tb| {
            let p = shifted(vertex.uv, [0, tb], periods);
            self.lower.is_none_or(|ring| above(ring, 0, p, periods))
                && self
                    .upper
                    .is_none_or(|ring| !above(ring, self.upper_turns_b, p, periods))
        })
    }

    /// Every piece a seam may be made of, ordered by the station it
    /// leaves and then by length; an error when the triangulation does
    /// not converge.
    fn bridges(
        &self,
        stations: &[Station],
        low_end: usize,
        high_end: usize,
    ) -> Result<Option<Vec<Bridge>>, &'static str> {
        let mut slow = false;
        let bridges = self.laid_out_bridges(stations, low_end, high_end, &mut slow);
        if slow {
            return Err(SLOW_TRIANGULATION);
        }
        Ok(bridges)
    }

    /// [`Self::bridges`]; `slow` is set when the triangulation does not
    /// converge.
    fn laid_out_bridges(
        &self,
        stations: &[Station],
        low_end: usize,
        high_end: usize,
        slow: &mut bool,
    ) -> Option<Vec<Bridge>> {
        let periods = self.param.periods();
        // Parameters scaled by how fast the surface moves with them, so
        // that short edges are short in space.
        let mut scale = [0.0_f64; 2];
        for v in stations.iter().flat_map(|s| &s.ring.verts) {
            let (_, [da, db]) = self.param.derivatives(v.uv);
            let [sa, sb] = scale;
            scale = [sa.max(da.length()), sb.max(db.length())];
        }
        let scale = scale.map(|s| if s.is_finite() && s > 0.0 { s } else { 1.0 });

        struct Point {
            plane: P2,
            station: usize,
            vertex: usize,
            ta: i32,
            usable: bool,
        }
        let origin = stations.first()?.ring.verts.first()?.uv;
        let mut points: Vec<Point> = Vec::new();
        let mut offsets: Vec<usize> = Vec::with_capacity(stations.len());
        let (mut low, mut high) = (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for (station, s) in stations.iter().enumerate() {
            offsets.push(points.len());
            for ta in -ROUTE_TURNS..=ROUTE_TURNS {
                for (vertex, v) in s.ring.verts.iter().enumerate() {
                    let plane = to_plane(shifted(v.uv, [ta, s.tb], periods), origin, scale);
                    if !(plane.0.is_finite() && plane.1.is_finite()) {
                        return None;
                    }
                    low = (low.0.min(plane.0), low.1.min(plane.1));
                    high = (high.0.max(plane.0), high.1.max(plane.1));
                    points.push(Point {
                        plane,
                        station,
                        vertex,
                        ta,
                        usable: v.free.is_none(),
                    });
                }
            }
        }
        let count = points.len();
        let mut cdt = Cdt::new(low, high, count.checked_add(8)?).ok()?;
        // In an order that jumps around, as for the face itself.
        let mut stride = ((count as f64) * 0.618) as usize;
        let coprime = |mut a: usize, mut b: usize| {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a == 1
        };
        while stride > 1 && !coprime(stride, count) {
            stride -= 1;
        }
        let stride = stride.max(1);
        let mut id_of: Vec<Option<u32>> = vec![None; count];
        // Per vertex of the triangulation, the first usable point there.
        let mut point_of: Vec<Option<usize>> = Vec::new();
        for n in 0..count {
            let i = n.checked_mul(stride)? % count;
            let Some(point) = points.get(i) else {
                continue;
            };
            // A point too close to another to tell them apart is left out.
            let id = match cdt.insert_boundary(point.plane) {
                Ok(id) => id,
                Err(CdtError::TooSlow) => {
                    *slow = true;
                    return None;
                }
                Err(_) => continue,
            };
            if let Some(slot) = id_of.get_mut(i) {
                *slot = Some(id);
            }
            let Some(k) = id.checked_sub(FIRST_VERTEX).map(|k| k as usize) else {
                continue;
            };
            if point_of.len() <= k {
                point_of.resize(k + 1, None);
            }
            if let Some(slot) = point_of.get_mut(k)
                && slot.is_none()
                && point.usable
            {
                *slot = Some(i);
            }
        }
        // The loops' segments. One that cannot be made an edge (it
        // crosses another: chords too coarse) is left out; the pieces are
        // tested against the loops themselves anyway.
        let copies = usize::try_from(2 * ROUTE_TURNS + 1).ok()?;
        for (station, s) in stations.iter().enumerate() {
            let n = s.ring.verts.len();
            let [wa, _] = s.ring.winding;
            let base = offsets.get(station).copied()?;
            for copy in 0..copies {
                for i in 0..n {
                    let next = if i + 1 < n {
                        Some(copy * n + i + 1)
                    } else {
                        // The loop closes in the copy it winds into.
                        let closing = i64::try_from(copy).ok()? + i64::from(wa);
                        usize::try_from(closing)
                            .ok()
                            .filter(|c| *c < copies)
                            .map(|c| c * n)
                    };
                    let Some(next) = next else { continue };
                    let ids = (
                        id_of.get(base + copy * n + i).copied().flatten(),
                        id_of.get(base + next).copied().flatten(),
                    );
                    if let (Some(p), Some(q)) = ids
                        && p != q
                        && cdt.constrain(p, q) == Err(CdtError::TooSlow)
                    {
                        *slow = true;
                        return None;
                    }
                }
            }
        }

        let mut bridges: Vec<Bridge> = Vec::new();
        let point_at = |v: u32| -> Option<&Point> {
            let k = v.checked_sub(FIRST_VERTEX)? as usize;
            points.get(point_of.get(k).copied().flatten()?)
        };
        for t in 0..cdt.triangle_count() {
            let Some(view) = cdt.triangle(t) else {
                continue;
            };
            let [x, y, z] = view.v;
            for (v, w) in [(x, y), (y, z), (z, x)] {
                let (Some(p), Some(q)) = (point_at(v), point_at(w)) else {
                    continue;
                };
                if p.station == q.station {
                    continue;
                }
                let length = (p.plane.0 - q.plane.0).hypot(p.plane.1 - q.plane.1);
                for (p, q) in [(p, q), (q, p)] {
                    bridges.push(Bridge {
                        from: p.station,
                        to: q.station,
                        leave: Some(p.vertex),
                        enter: Some(q.vertex),
                        dt: q.ta.saturating_sub(p.ta),
                        length,
                    });
                }
            }
        }
        // Where no loop winds, the surface ends in a point that every
        // loop sees from its lowest (or highest) vertex unless another
        // loop is in the way.
        let [_, (b_low, b_high)] = self.param.domain();
        let [_, sb] = scale;
        for (station, s) in stations.iter().enumerate() {
            let proper = || {
                s.ring
                    .verts
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.free.is_none())
            };
            let b_of = |v: &Bv| get2(v.uv, 1);
            if self.lower.is_none()
                && let Some((i, v)) =
                    proper().min_by(|x, y| b_of(x.1).total_cmp(&b_of(y.1)).then(x.0.cmp(&y.0)))
            {
                bridges.push(Bridge {
                    from: low_end,
                    to: station,
                    leave: None,
                    enter: Some(i),
                    dt: 0,
                    length: ((b_of(v) - b_low) * sb).abs(),
                });
            }
            if self.upper.is_none()
                && let Some((i, v)) =
                    proper().max_by(|x, y| b_of(x.1).total_cmp(&b_of(y.1)).then(y.0.cmp(&x.0)))
            {
                bridges.push(Bridge {
                    from: station,
                    to: high_end,
                    leave: Some(i),
                    enter: None,
                    dt: 0,
                    length: ((b_high - b_of(v)) * sb).abs(),
                });
            }
        }
        let key = |b: &Bridge| (b.from, b.to, b.leave, b.enter, b.dt);
        bridges.sort_by(|x, y| {
            x.from
                .cmp(&y.from)
                .then(x.length.total_cmp(&y.length))
                .then(key(x).cmp(&key(y)))
        });
        bridges.dedup_by(|x, y| key(x) == key(y));
        Some(bridges)
    }

    /// The seam in pieces, if there is a way for one; an error when the
    /// triangulation of the loops does not converge.
    fn cut(&self) -> Result<Option<Cut>, &'static str> {
        let periods = self.param.periods();
        let mut stations: Vec<Station> = Vec::new();
        let (mut source, mut target) = (None, None);
        if let Some(ring) = self.lower {
            source = Some(stations.len());
            stations.push(Station {
                ring,
                hole: None,
                tb: 0,
            });
        }
        if let Some(ring) = self.upper {
            target = Some(stations.len());
            stations.push(Station {
                ring,
                hole: None,
                tb: self.upper_turns_b,
            });
        }
        for &(index, ring) in self.holes {
            if let Some(tb) = self.hole_turns(ring, periods) {
                stations.push(Station {
                    ring,
                    hole: Some(index),
                    tb,
                });
            }
        }
        // The two ends of the surface come after the loops.
        let low_end = stations.len();
        let high_end = low_end + 1;
        let (source, target) = (source.unwrap_or(low_end), target.unwrap_or(high_end));
        let Some(bridges) = self.bridges(&stations, low_end, high_end)? else {
            return Ok(None);
        };
        Ok(self.route(&stations, &bridges, source, target))
    }

    /// The shortest chain of `bridges` from station `source` to station
    /// `target`, as a seam.
    fn route(
        &self,
        stations: &[Station],
        bridges: &[Bridge],
        source: usize,
        target: usize,
    ) -> Option<Cut> {
        let periods = self.param.periods();
        let [_, (b_low, b_high)] = self.param.domain();
        let high_end = stations.len() + 1;

        // A piece in parameters, when the station it leaves is moved by
        // `turns` periods.
        let piece = |bridge: &Bridge, turns: i32| -> Option<[Uv; 2]> {
            let at = |station: usize, vertex: Option<usize>, turns: i32| -> Option<Uv> {
                let s = stations.get(station)?;
                Some(shifted(
                    s.ring.verts.get(vertex?)?.uv,
                    [turns, s.tb],
                    periods,
                ))
            };
            let leave = at(bridge.from, bridge.leave, turns);
            let enter = at(bridge.to, bridge.enter, turns.checked_add(bridge.dt)?);
            let ends = match (leave, enter) {
                (Some(p), Some(q)) => [p, q],
                (None, Some([a, b])) => [[a, b_low], [a, b]],
                (Some([a, b]), None) => [[a, b], [a, b_high]],
                (None, None) => return None,
            };
            ends.iter().flatten().all(|x| x.is_finite()).then_some(ends)
        };

        // Breadth first from the lower side: the fewest pieces, and among
        // those the short ones first.
        let mut reached: Vec<Option<Reached>> = vec![None; high_end + 1];
        let lower_turns = match self.lower.map(|ring| ring.winding) {
            Some([wa, _]) if wa < 0 => -1,
            _ => 0,
        };
        if let Some(slot) = reached.get_mut(source) {
            *slot = Some(Reached {
                by: None,
                turns: lower_turns,
                piece: None,
            });
        }
        let crosses_the_seam = |ends: [Uv; 2], from: usize, reached: &[Option<Reached>]| -> bool {
            let [p, q] = ends;
            let mut node = from;
            for _ in 0..reached.len() {
                let Some(Some(step)) = reached.get(node) else {
                    return false;
                };
                let (Some([c, d]), Some(by)) = (step.piece, step.by) else {
                    return false;
                };
                for k in -1..=1 {
                    let (c, d) = (shifted(c, [k, 0], periods), shifted(d, [k, 0], periods));
                    if seam_blocked(p, q, c, d) {
                        return true;
                    }
                }
                let Some(bridge) = bridges.get(by) else {
                    return false;
                };
                node = bridge.from;
            }
            false
        };
        let mut queue = std::collections::VecDeque::from([source]);
        while let Some(node) = queue.pop_front() {
            if node == target {
                break;
            }
            let Some(Some(here)) = reached.get(node).copied() else {
                continue;
            };
            let first = bridges.partition_point(|b| b.from < node);
            for (index, bridge) in bridges.iter().enumerate().skip(first) {
                if bridge.from != node {
                    break;
                }
                if !matches!(reached.get(bridge.to), Some(None)) {
                    continue;
                }
                let Some(ends) = piece(bridge, here.turns) else {
                    continue;
                };
                let [p, q] = ends;
                // (No cap on the pieces found to cross: a station is
                // queued once (it must be unreached), so each bridge is
                // tried at most once, and the bridges are the sides of a
                // triangulation of the stations' vertices (`bridges`: a
                // `Cdt` of `count` points, `count` the vertices times the
                // turns tried), at most three per point: the work is
                // proportional to the stations' vertices. A cap of 256
                // refused a pipe with ten holes at a fine chord, which has
                // more.)
                if crosses_the_seam(ends, node, &reached) || (self.crosses_a_loop)(p, q) {
                    continue;
                }
                if let Some(slot) = reached.get_mut(bridge.to) {
                    *slot = Some(Reached {
                        by: Some(index),
                        turns: here.turns.checked_add(bridge.dt)?,
                        piece: Some(ends),
                    });
                    queue.push_back(bridge.to);
                }
            }
        }

        // Back from the upper side along the bridges that reached it.
        let mut chain: Vec<(Bridge, Reached)> = Vec::new();
        let mut node = target;
        for _ in 0..reached.len() {
            let step = reached.get(node).copied().flatten()?;
            let Some(by) = step.by else { break };
            let bridge = bridges.get(by).copied()?;
            chain.push((bridge, step));
            node = bridge.from;
        }
        if node != source || chain.is_empty() {
            return None;
        }
        chain.reverse();
        let mut cut = Cut {
            a: 0.0,
            upper_a: 0.0,
            lower_start: None,
            upper_start: None,
            via: Vec::new(),
        };
        let mut entered: Option<usize> = None;
        for (bridge, step) in &chain {
            let [[from_a, _], [to_a, _]] = step.piece?;
            if bridge.from == source {
                match bridge.leave {
                    Some(start) => cut.lower_start = Some(start),
                    None => cut.a = from_a,
                }
            } else {
                let station = stations.get(bridge.from)?;
                cut.via.push(Via {
                    ring: station.hole?,
                    turns: [step.turns.checked_sub(bridge.dt)?, station.tb],
                    enter: entered?,
                    leave: bridge.leave?,
                });
            }
            if bridge.to == target {
                match (bridge.enter, self.upper.map(|ring| ring.winding)) {
                    (Some(start), Some([wa, _])) => {
                        // `Region::add_chain` puts the left end of a loop
                        // that runs backwards a period before its start.
                        let turns = step.turns.checked_add(i32::from(wa < 0))?;
                        cut.upper_start = Some((start, turns));
                    }
                    _ => cut.upper_a = to_a,
                }
            }
            entered = bridge.enter;
        }
        Some(cut)
    }
}

/// A whole torus: a rectangle of one period by one period, cut along both
/// seams. All four corners are one point.
fn torus_region(param: &Param, period_a: f64, period_b: f64) -> Result<Region, TessError> {
    let [(a, _), (b, _)] = param.domain();
    let corner = |uv: Uv| Bv {
        uv,
        pos: param.eval([a, b]),
        free: None,
        chord: None,
    };
    let mut region = Region::default();
    let c00 = region.add(corner([a, b]));
    let c10 = region.add(corner([a + period_a, b]));
    let c01 = region.add(corner([a, b + period_b]));
    let c11 = region.add(corner([a + period_a, b + period_b]));
    region.seams.push(Seam {
        from: c00,
        to: c01,
        twin_from: c10,
        twin_to: c11,
        shift: [period_a, 0.0],
    });
    region.seams.push(Seam {
        from: c00,
        to: c10,
        twin_from: c01,
        twin_to: c11,
        shift: [0.0, period_b],
    });
    Ok(region)
}

/// Grid cells for a turn of `sweep` radians at `radius`, as for an arc
/// but never more than a quarter turn each: a closed surface must keep
/// some volume however coarse the tolerance.
fn arc_cells(
    radius: f64,
    sweep: f64,
    tolerance: f64,
    params: &TessParams,
    face: usize,
) -> Result<u32, TessError> {
    let least = (sweep.abs() / FRAC_PI_2 - 1e-9).ceil().max(1.0);
    if exceeds(least, f64::from(params.max_segments) * 8.0) {
        return Err(TessError::TooFine {
            patch: face,
            needed_segments: least,
            max_segments: params.max_segments,
        });
    }
    if !positive(radius) {
        // In range by the check above.
        return Ok(least as u32);
    }
    let with_tolerance = TessParams {
        chordal_tolerance: tolerance,
        ..*params
    };
    let cells = arc_segments(radius, sweep, &with_tolerance).map_err(|e| match e {
        TessError::TooFine {
            needed_segments,
            max_segments,
            ..
        } => TessError::TooFine {
            patch: face,
            needed_segments,
            max_segments,
        },
        other => other,
    })?;
    // In range by the check above.
    Ok(cells.max(least as u32))
}

/// The cells of a grid over a sphere or a torus, in `u` (around the axis)
/// and in `v`, from what each direction needs (`cells_u`, `cells_v`,
/// given the parameters to size by).
///
/// With the angle criterion, a cell of the grid is cut in two along a
/// diagonal, over which the surface's normal turns by both of its sides
/// at once: about `√(tu² + tv²)` for turns `tu` and `tv` along them (the
/// normal of either surface turns by at most `du` around the axis, by
/// `dv` along a meridian, and the two turns are square to each other).
/// Each side held to the angle, the diagonal was not, and the refinement
/// cut every diagonal of the grid. So `v` gets what `u` leaves of the
/// angle: `tv = √(angle² − tu²)`, with `u` held to `angle / √2` where it
/// needs more (otherwise, as before, its own count: the chordal
/// tolerance usually asks for more cells around a ring than the angle
/// does).
///
/// A side shorter than the floor (`ANGLE_FLOOR` tolerances) is not held
/// to the angle, and `cells_v` leaves the angle out where the side across
/// would be that short (a fillet thinner than the tolerance: two cells
/// across the tube). But the diagonal is as long as the side around the
/// axis (`radius_u` times the step in `u`) and, where that is not below
/// the floor, is held to the angle: it turned by the tube's whole
/// crossing and the refinement cut it until its length around was below
/// the floor, a dozen points per cell. There `v` gets the cells the angle
/// asks for, floor or no floor (`span_v / tv`, as many as the angle
/// gives a whole turn at most, and no more than the grid's limit): a
/// count of the angle alone, not of a length.
fn both_ways(
    cells_u: impl Fn(&TessParams) -> Result<u32, TessError>,
    cells_v: impl Fn(&TessParams) -> Result<u32, TessError>,
    (radius_u, span_u): (f64, f64),
    span_v: f64,
    params: &TessParams,
) -> Result<(u32, u32), TessError> {
    let Some(angle) = params.max_angle else {
        return Ok((cells_u(params)?, cells_v(params)?));
    };
    let mut nu = cells_u(params)?;
    let mut turn_u = span_u.abs() / f64::from(nu.max(1));
    let most = angle * std::f64::consts::FRAC_1_SQRT_2;
    if turn_u > most {
        nu = cells_u(&TessParams {
            max_angle: Some(most),
            ..*params
        })?;
        turn_u = span_u.abs() / f64::from(nu.max(1));
    }
    let left = (angle * angle - turn_u * turn_u).max(0.0).sqrt().max(most);
    let mut nv = cells_v(&TessParams {
        max_angle: Some(left),
        ..*params
    })?;
    let side_u = radius_u.abs() * turn_u;
    let by_angle = (span_v.abs() / left - 1e-9).ceil();
    let limit = f64::from(params.max_segments);
    if side_u >= crate::ANGLE_FLOOR * params.chordal_tolerance
        && by_angle > f64::from(nv)
        && by_angle <= limit
    {
        // In range by the check above.
        nv = by_angle as u32;
    }
    Ok((nu, nv))
}

/// How many grid cells the box `low..high` of the surface's own `(u, v)`
/// needs in each direction for a cell of the grid, cut in two triangles,
/// to be within the tolerance of the surface: by the arc for the analytic
/// kinds, by bounds on the second derivatives for a Bézier patch (its
/// control net's, over the whole patch) and for a spline surface (proven
/// ones over the box, [`spline_second_bounds`]; until 2026-10-14 sampled
/// second differences times 1.5, which promised nothing). The triangles
/// are not all cells of the grid: those along the boundary are not, and
/// grid points too near the boundary are left out. So every triangle is
/// measured afterwards all the same.
fn seed_cells(
    surface: &Surface,
    closure: &Closure,
    low: (f64, f64),
    high: (f64, f64),
    params: &TessParams,
    face: usize,
) -> Result<(u32, u32), TessError> {
    let tolerance = params.chordal_tolerance;
    let (span_u, span_v) = (high.0 - low.0, high.1 - low.1);
    // On a surface curved in both directions a grid cell sags in u and in
    // v at once, and inside a triangle the two add up. Giving each
    // direction half the tolerance keeps their sum within it.
    let half = tolerance / 2.0;
    let from_second_derivatives = |muu: f64, muv: f64, mvv: f64| -> Result<(u32, u32), TessError> {
        // Filip, Magedson & Markot, "Surface algorithms using bounds on
        // derivatives" (CAGD 3, 1986): linear interpolation over a right
        // triangle with legs `h` in parameter space deviates by at most
        // `(Muu + 2 Muv + Mvv) h² / 8`.
        let per_unit = ((muu + 2.0 * muv + mvv) / (8.0 * tolerance)).sqrt();
        let (nu, nv) = ((per_unit * span_u).ceil(), (per_unit * span_v).ceil());
        let limit = f64::from(params.max_segments);
        if !(nu <= limit && nv <= limit) {
            return Err(TessError::TooFine {
                patch: face,
                needed_segments: nu.max(nv),
                max_segments: params.max_segments,
            });
        }
        // In range by the check above.
        Ok(((nu as u32).max(1), (nv as u32).max(1)))
    };
    match surface {
        Surface::Plane(_) => Ok((1, 1)),
        Surface::Cylinder(c) => Ok((arc_cells(c.radius(), span_u, tolerance, params, face)?, 1)),
        Surface::Cone(c) => {
            let slope = c.half_angle().tan();
            let radius = (c.radius() + low.1 * slope)
                .abs()
                .max((c.radius() + high.1 * slope).abs());
            Ok((arc_cells(radius, span_u, tolerance, params, face)?, 1))
        }
        Surface::Sphere(s) => {
            let widest = if low.1 <= 0.0 && high.1 >= 0.0 {
                1.0
            } else {
                low.1.cos().max(high.1.cos()).max(0.0)
            };
            both_ways(
                |p| arc_cells(s.radius() * widest, span_u, half, p, face),
                |p| arc_cells(s.radius(), span_v, half, p, face),
                (s.radius() * widest, span_u),
                span_v,
                params,
            )
        }
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            both_ways(
                |p| arc_cells(major + minor, span_u, half, p, face),
                |p| arc_cells(minor, span_v, half, p, face),
                (major + minor, span_u),
                span_v,
                params,
            )
        }
        Surface::Bezier(b) => {
            let (muu, muv, mvv) = b.second_derivative_bounds();
            from_second_derivatives(muu, muv, mvv)
        }
        Surface::Nurbs(spline) => {
            let (muu, muv, mvv) = spline_second_bounds(spline, closure, low, high);
            let (nu, nv) = spline_cells(
                (muu + muv, mvv + muv),
                (span_u, span_v),
                tolerance,
                f64::from(params.max_segments),
            )
            .map_err(|needed_segments| TessError::TooFine {
                patch: face,
                needed_segments,
                max_segments: params.max_segments,
            })?;
            // In range: `spline_cells` keeps within the limit.
            Ok(((nu as u32).max(1), (nv as u32).max(1)))
        }
    }
}

/// The cells of the grid over a face on a spline surface, in u and in v:
/// whole numbers, at most `limit` each, or as an error the number that
/// would be needed.
///
/// `(a, b)` are `(Muu + Muv, Mvv + Muv)` from bounds on the second
/// derivatives over the face's box, which is `span` wide. Over a right
/// triangle with legs `hu` and `hv` along the parameters a surface is
/// within `(Muu hu² + 2 Muv hu hv + Mvv hv²) / 8` of the plane through
/// its corners (Filip, Magedson & Markot 1986, as for a Bézier patch
/// above), and `2 hu hv ≤ hu² + hv²`, so the cells do when
///
/// `a hu² + b hv² ≤ 8 × tolerance`.
///
/// Each parameter gets half of that: `a hu² = b hv² = 4 × tolerance`. A
/// surface far more curved one way than the other is then not as fine
/// both ways as its tighter direction asks (the wall an extruded spline
/// sweeps is straight along the sweep: one cell), and the cells are
/// never more than with one step for both, the geometric mean against
/// the arithmetic one. Where one parameter would need more cells than
/// the limit and the other has room, it gets the limit and the other
/// what is then left of the sum: finer there, up to a factor √2 in
/// reach.
fn spline_cells(
    (a, b): (f64, f64),
    (span_u, span_v): (f64, f64),
    tolerance: f64,
    limit: f64,
) -> Result<(f64, f64), f64> {
    let cells = |bound: f64, span: f64, share: f64| (bound / share).sqrt() * span;
    let even = 4.0 * tolerance;
    let (nu, nv) = (cells(a, span_u, even).ceil(), cells(b, span_v, even).ceil());
    let needed = nu.max(nv);
    // (Negated comparisons: a number that is not one is over the limit.)
    match (nu <= limit, nv <= limit) {
        (true, true) => Ok((nu, nv)),
        (false, true) => {
            let left = 8.0 * tolerance - a * (span_u / limit) * (span_u / limit);
            let nv = cells(b, span_v, left).ceil();
            if left > 0.0 && nv <= limit {
                Ok((limit, nv))
            } else {
                Err(needed)
            }
        }
        (true, false) => {
            let left = 8.0 * tolerance - b * (span_v / limit) * (span_v / limit);
            let nu = cells(a, span_u, left).ceil();
            if left > 0.0 && nu <= limit {
                Ok((nu, limit))
            } else {
                Err(needed)
            }
        }
        (false, false) => Err(needed),
    }
}

/// The least part of a knot span a patch is cut down to before its
/// derivatives are bounded: the differences of control points that the
/// bounds are made of lose their digits on a patch much narrower.
const LEAST_PART: f64 = 1.0 / 16.0;

/// How often a patch is cut in halves before its derivatives are bounded
/// ([`patch_second_bounds`]). Measured on surfaces of revolution, whose
/// rational arcs around the axis are where the hull of the control net
/// is loosest (`tests::the_bounds_on_a_spline_surface_are_above_its_second_differences`):
/// uncut, the bounds are 2.7 to 4.7 times the largest second
/// differences; cut three times 1.6 to 2.4 times, four times 1.5 to 2.1
/// times, six times 1.4 to 1.8 times (on a polynomial surface 1.2 times,
/// cut or not). The cells of the grid go by the root of that. Every cut
/// doubles the pieces, and both ways of cutting are tried: 60 nets are
/// bounded for four cuts, 252 for six.
const HALVINGS: u32 = 4;

/// Bounds `(Muu, Muv, Mvv)` on the second derivatives of `patch`, which
/// is a part of a surface `width` wide in the surface's parameters.
///
/// The bound of a patch is that of the hull of its control net
/// (`RationalBezierPatch::derivative_bounds`), which is the nearer to
/// the patch the smaller the patch is; so the patch is cut in halves
/// [`HALVINGS`] times, each time across the parameter in which that
/// brings the bounds down most (a surface of revolution gains by being
/// cut around its axis, where its weights vary, and nothing along it),
/// and the bound is the largest over the pieces. Every round bounds the
/// same patch, so the smallest bound of all rounds stands.
fn patch_second_bounds(
    patch: craft3d_geom::RationalBezierPatch,
    width: (f64, f64),
) -> Option<(f64, f64, f64)> {
    use craft3d_geom::RationalBezierPatch;
    let over = |pieces: &[RationalBezierPatch], width: (f64, f64)| -> Option<(f64, f64, f64)> {
        let (mut uu, mut uv, mut vv) = (0.0_f64, 0.0_f64, 0.0_f64);
        for piece in pieces {
            let bounds = piece.derivative_bounds()?.over(width.0, width.1);
            uu = uu.max(bounds.second_uu);
            uv = uv.max(bounds.second_uv);
            vv = vv.max(bounds.second_vv);
        }
        Some((uu, uv, vv))
    };
    // What the cells of the grid go by.
    let weight = |(uu, uv, vv): (f64, f64, f64)| uu + 2.0 * uv + vv;
    let mut pieces = vec![patch];
    let mut width = width;
    let mut best = over(&pieces, width)?;
    for _ in 0..HALVINGS {
        let in_u: Vec<RationalBezierPatch> = pieces
            .iter()
            .flat_map(|piece| {
                let (a, b) = piece.split_u(0.5);
                [a, b]
            })
            .collect();
        let in_v: Vec<RationalBezierPatch> = pieces
            .iter()
            .flat_map(|piece| {
                let (a, b) = piece.split_v(0.5);
                [a, b]
            })
            .collect();
        let (half_u, half_v) = ((width.0 / 2.0, width.1), (width.0, width.1 / 2.0));
        let (bound_u, bound_v) = (over(&in_u, half_u)?, over(&in_v, half_v)?);
        let bound = if weight(bound_u) <= weight(bound_v) {
            (pieces, width) = (in_u, half_u);
            bound_u
        } else {
            (pieces, width) = (in_v, half_v);
            bound_v
        };
        best = (
            best.0.min(bound.0),
            best.1.min(bound.1),
            best.2.min(bound.2),
        );
    }
    Some(best)
}

/// The stretches of `low..high` as they lie in the range `from..to` of a
/// parameter: the one stretch within the range, or, when the surface
/// closes in this parameter (`wrap`), the stretches of each turn brought
/// back into the range.
fn stretches(
    low: f64,
    high: f64,
    (from, to): (f64, f64),
    wrap: Option<(f64, f64)>,
) -> Vec<(f64, f64)> {
    let whole = vec![(from, to)];
    let period = to - from;
    if !(low.is_finite() && high.is_finite() && period.is_finite() && period > 0.0) {
        return whole;
    }
    if wrap.is_none() {
        let (a, b) = (low.max(from), high.min(to));
        return if b > a { vec![(a, b)] } else { whole };
    }
    let (first, last) = (
        ((low - from) / period).floor(),
        ((high - from) / period).floor(),
    );
    // More than a turn and a bit: all of it.
    if exceeds(last - first, 2.0) || exceeds(high - low, period) {
        return whole;
    }
    let mut parts = Vec::new();
    let mut turn = first;
    while turn <= last {
        let start = from + turn * period;
        let (a, b) = (
            low.max(start) - turn * period,
            high.min(start + period) - turn * period,
        );
        if b > a {
            parts.push((a.max(from), b.min(to)));
        }
        turn += 1.0;
    }
    if parts.is_empty() { whole } else { parts }
}

/// Upper bounds `(Muu, Muv, Mvv)` on the lengths of the second partial
/// derivatives of a spline surface over the box `low..high` of its
/// parameters: proven ones, not sampled. The surface is a rational Bézier
/// patch on each of its knot spans (`NurbsSurface::bezier_spans`); each
/// patch the box reaches is cut down to the part the box covers, and the
/// derivatives of that part are bounded from its control net
/// (`RationalBezierPatch::derivative_bounds`: the hull of the net bounds
/// the patch and its weights). The bound is the largest over the parts.
///
/// A part is never cut narrower than [`LEAST_PART`] of its span; a wider
/// part bounds a narrower one. Each part is bounded in [`HALVINGS`]
/// rounds of halves, which brings the bound nearer to the surface. Infinite when no bound is found (a span
/// that cannot be split out), which ends as `TessError::TooFine`.
fn spline_second_bounds(
    surface: &craft3d_geom::NurbsSurface,
    closure: &Closure,
    low: (f64, f64),
    high: (f64, f64),
) -> (f64, f64, f64) {
    let unbounded = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let Ok(spans) = surface.bezier_spans() else {
        return unbounded;
    };
    let (domain_u, domain_v) = surface.domain();
    let [wrap_u, wrap_v] = closure.wrap;
    let across_u = stretches(low.0, high.0, domain_u, wrap_u);
    let across_v = stretches(low.1, high.1, domain_v, wrap_v);
    // The part `0..1` of a span that a stretch covers, at least
    // `LEAST_PART` wide; whole when it is all but whole.
    let part = |span: (f64, f64), stretch: (f64, f64)| -> Option<(f64, f64)> {
        let width = span.1 - span.0;
        let (a, b) = (stretch.0.max(span.0), stretch.1.min(span.1));
        if !(b > a && width > 0.0) {
            return None;
        }
        let (mut s0, mut s1) = ((a - span.0) / width, (b - span.0) / width);
        if s1 - s0 < LEAST_PART {
            s0 = (0.5 * (s0 + s1 - LEAST_PART)).clamp(0.0, 1.0 - LEAST_PART);
            s1 = s0 + LEAST_PART;
        }
        const ALL_BUT: f64 = 1e-9;
        Some((
            if s0 < ALL_BUT { 0.0 } else { s0 },
            if s1 > 1.0 - ALL_BUT { 1.0 } else { s1 },
        ))
    };
    let (mut muu, mut muv, mut mvv) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut found = false;
    for span in &spans {
        for &stretch_u in &across_u {
            let Some((s0, s1)) = part(span.u, stretch_u) else {
                continue;
            };
            for &stretch_v in &across_v {
                let Some((t0, t1)) = part(span.v, stretch_v) else {
                    continue;
                };
                // The patch over `s0..s1` by `t0..t1`.
                let mut patch = span.patch.clone();
                if s0 > 0.0 {
                    patch = patch.split_u(s0).1;
                }
                if s1 < 1.0 {
                    patch = patch.split_u((s1 - s0) / (1.0 - s0)).0;
                }
                if t0 > 0.0 {
                    patch = patch.split_v(t0).1;
                }
                if t1 < 1.0 {
                    patch = patch.split_v((t1 - t0) / (1.0 - t0)).0;
                }
                let width = (
                    (span.u.1 - span.u.0) * (s1 - s0),
                    (span.v.1 - span.v.0) * (t1 - t0),
                );
                let Some((uu, uv, vv)) = patch_second_bounds(patch, width) else {
                    return unbounded;
                };
                muu = muu.max(uu);
                muv = muv.max(uv);
                mvv = mvv.max(vv);
                found = true;
            }
        }
    }
    if found { (muu, muv, mvv) } else { unbounded }
}

/// How far `target` is from the surface, starting the search at `uv`.
/// Never more than the distance to the surface point at `uv` itself.
fn distance_to_surface(param: &Param, uv: Uv, target: Vec3) -> f64 {
    let at_uv = param.eval(uv).distance(target);
    let found = if param.is_freeform() {
        // A few Gauss–Newton steps on |S(u, v) − target|² from the guess.
        // Held within the domain, but for a parameter the surface closes
        // in: there the search goes on across the seam.
        let [(a0, a1), (b0, b1)] = param.domain();
        let [period_a, period_b] = param.periods();
        let (a0, a1) = if period_a.is_some() {
            (f64::NEG_INFINITY, f64::INFINITY)
        } else {
            (a0, a1)
        };
        let (b0, b1) = if period_b.is_some() {
            (f64::NEG_INFINITY, f64::INFINITY)
        } else {
            (b0, b1)
        };
        let [mut a, mut b] = uv;
        let mut best = at_uv;
        for _ in 0..8 {
            let (s, [da, db]) = param.derivatives([a, b]);
            let r = target - s;
            let (e, f, g) = (da.dot(da), da.dot(db), db.dot(db));
            let det = e * g - f * f;
            if !(det.is_finite() && det > 1e-14 * e * g) {
                break;
            }
            let (ra, rb) = (r.dot(da), r.dot(db));
            let next_a = (a + (g * ra - f * rb) / det).clamp(a0, a1);
            let next_b = (b + (e * rb - f * ra) / det).clamp(b0, b1);
            let distance = param.eval([next_a, next_b]).distance(target);
            if !positive(best - distance) {
                break;
            }
            (a, b, best) = (next_a, next_b, distance);
        }
        best
    } else {
        // Closed form for the analytic kinds.
        match param.surface.project(target) {
            Ok((u, v)) => param.surface.eval(u, v).distance(target),
            Err(_) => at_uv,
        }
    };
    if found < at_uv { found } else { at_uv }
}

/// The part of (the size of its coordinates times its longest side) below
/// which twice the area of a triangle is rounding: some thousands of the
/// unit in the last place of the products it is the difference of.
const ROUNDED_AWAY: f64 = 1e-12;

/// What to do with a triangle that is not good enough.
enum Action {
    Keep,
    SplitEdge(usize),
    SplitTriangle,
    /// Within the tolerance, but the normals at the ends of this free
    /// side, not shorter than the floor, are further apart than the angle
    /// criterion allows: the side to cut. Where no point fits there, the
    /// triangle is kept as it is.
    Turn(usize),
}

/// The triangulation of one face while it is refined.
struct Work<'a> {
    param: Param<'a>,
    cdt: Cdt,
    /// Per triangulation vertex (after the bounding square's four).
    uv: Vec<Uv>,
    pos: Vec<Vec3>,
    free: Vec<Option<usize>>,
    /// Parameters are scaled about `centre` for the triangulation, or
    /// mapped by `warp`.
    centre: Uv,
    scale: Uv,
    warp: Option<[Warp; 2]>,
    tolerance: f64,
    /// `TessParams::max_angle`, and the shortest side held to it.
    angle: Option<f64>,
    angle_floor: f64,
    /// Per vertex, the surface's normal there, with the angle criterion.
    normals: Vec<Option<Vec3>>,
}

impl Work<'_> {
    fn plane(&self, uv: Uv) -> P2 {
        match &self.warp {
            Some([wa, wb]) => {
                let [a, b] = uv;
                (wa.map(a), wb.map(b))
            }
            None => to_plane(uv, self.centre, self.scale),
        }
    }

    fn vertex(&self, v: u32) -> Option<(Uv, Vec3, Option<usize>)> {
        let i = v.checked_sub(FIRST_VERTEX)? as usize;
        Some((*self.uv.get(i)?, *self.pos.get(i)?, *self.free.get(i)?))
    }

    /// Records the data of a vertex the triangulation has just made.
    fn record(&mut self, v: u32, uv: Uv, pos: Vec3, free: Option<usize>) -> Result<(), CdtError> {
        if v.checked_sub(FIRST_VERTEX).map(|i| i as usize) != Some(self.uv.len()) {
            return Err(CdtError::Internal);
        }
        if self.angle.is_some() {
            self.normals.push(self.param.normal(uv));
        }
        self.uv.push(uv);
        self.pos.push(pos);
        self.free.push(free);
        Ok(())
    }

    /// The surface's normal at vertex `v`, kept from when it was made
    /// (with the angle criterion only).
    fn normal_of(&self, v: u32) -> Option<Vec3> {
        let i = v.checked_sub(FIRST_VERTEX)? as usize;
        *self.normals.get(i)?
    }

    /// The mean parameters of some vertices, leaving out for each
    /// parameter the vertices where it is free: a pole is at every
    /// longitude, so it takes the longitude of its neighbours.
    fn mean(corners: &[(Uv, Vec3, Option<usize>)]) -> Uv {
        let mut out = [0.0; 2];
        for k in 0..2 {
            let (mut fixed, mut fixed_count, mut all) = (0.0, 0.0, 0.0);
            for c in corners {
                let x = get2(c.0, k);
                all += x;
                if c.2 != Some(k) {
                    fixed += x;
                    fixed_count += 1.0;
                }
            }
            let value = if fixed_count > 0.0 {
                fixed / fixed_count
            } else {
                all / (corners.len().max(1) as f64)
            };
            set2(&mut out, k, value);
        }
        out
    }

    /// How far `target`, a point of a chord or a triangle, is from the
    /// surface. The surface point at the mean parameters `uv` is the
    /// first candidate. Only when that is too far is the nearest surface
    /// point looked for: the mean parameters are not where the surface is
    /// nearest when the parametrisation is far from uniform (a chord
    /// across a cone runs along a spiral in parameter space) or when the
    /// triangle lies along a boundary that is curved in parameter space.
    fn off_surface(&self, uv: Uv, target: Vec3) -> f64 {
        let off = self.param.eval(uv).distance(target);
        if off <= self.tolerance {
            off
        } else {
            distance_to_surface(&self.param, uv, target)
        }
    }

    /// Measures triangle `t` (see the module documentation).
    fn assess(&self, t: usize) -> Action {
        let Some(view) = self.cdt.triangle(t).filter(|view| view.inside) else {
            return Action::Keep;
        };
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| self.vertex(v)) else {
            return Action::Keep;
        };
        if a.1 == b.1 || b.1 == c.1 || c.1 == a.1 {
            return Action::Keep; // Collapsed at a pole: dropped later.
        }
        let corners = [a, b, c];
        let pair = |i: usize| -> [(Uv, Vec3, Option<usize>); 2] {
            let [x, y, z] = corners;
            match i % 3 {
                0 => [y, z],
                1 => [z, x],
                _ => [x, y],
            }
        };
        // Free edges: the surface at the edge's mean parameters against
        // the chord's midpoint.
        let mut worst: Option<(f64, usize)> = None;
        let [c0, c1, c2] = view.constrained;
        for (i, constrained) in [c0, c1, c2].into_iter().enumerate() {
            if constrained {
                continue;
            }
            let ends = pair(i);
            let [p, q] = ends;
            let off = self.off_surface(Self::mean(&ends), p.1.lerp(q.1, 0.5));
            // NaN counts as failing: it ends in an error, not a bad mesh.
            if exceeds(off, self.tolerance) && worst.is_none_or(|(w, _)| exceeds(off, w)) {
                worst = Some((off, i));
            }
        }
        if let Some((_, i)) = worst {
            return Action::SplitEdge(i);
        }
        let middle = Self::mean(&corners);
        let centroid = (a.1 + b.1 + c.1) * (1.0 / 3.0);
        let off = self.off_surface(middle, centroid);
        if exceeds(off, self.tolerance) {
            return Action::SplitTriangle;
        }
        // Facing the way the surface does. The surface normal at the mean
        // parameters decides when it is clear; otherwise the normal at the
        // nearest surface point does.
        let facing = (b.1 - a.1).cross(c.1 - a.1);
        // A triangle whose area is within the rounding of its own corners
        // faces no way that can be told: `facing` is then the difference
        // of products that agree to their last digits. Such triangles
        // lie where a boundary runs into a pole (between points of an
        // edge a millionth of the face's size from the pole, which is one
        // point however many vertices it has in parameter space); cut in
        // two they only get smaller, and there is nothing to mend. (On a
        // spline surface: at the apex of a cone or a pole of a sphere the
        // parameters are exact and such a triangle is mended by a cut.)
        let reach = [a.1, b.1, c.1].iter().fold(0.0_f64, |m, p| {
            m.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
        });
        let longest =
            a.1.distance(b.1)
                .max(b.1.distance(c.1))
                .max(c.1.distance(a.1));
        // But such a triangle longer than the tolerance is no rounding
        // near a pole: its corners lie on one line in space while the
        // surface between them does not (two points of a ruling of a
        // cone that a straight spline sweeps, and its apex, or another
        // point of that ruling). It faces no way either, and is cut as
        // one that faces the wrong way is, on any surface; so is one
        // with no area at all.
        let rounded = facing.length() <= ROUNDED_AWAY * reach * longest;
        let flat =
            !positive(facing.length_squared()) || (rounded && exceeds(longest, self.tolerance));
        if !flat && rounded && self.param.is_freeform() {
            return Action::Keep;
        }
        let (_, [da, db]) = self.param.derivatives(middle);
        let reference = da.cross(db);
        let agreement = facing.dot(reference);
        let clear = !flat && agreement > 0.5 * facing.length() * reference.length();
        if !clear {
            let exact = if flat {
                Some(0.0)
            } else if self.param.is_freeform() {
                (reference.length() > 0.0).then_some(agreement)
            } else {
                self.param
                    .surface
                    .project(centroid)
                    .ok()
                    .and_then(|(u, v)| self.param.surface.normal(u, v))
                    .map(|n| facing.dot(n) * self.param.handedness())
            };
            if exact.is_some_and(|x| !positive(x)) {
                // The longest free edge, in the scaled parameter plane.
                let mut longest: Option<(f64, usize)> = None;
                for (i, constrained) in [c0, c1, c2].into_iter().enumerate() {
                    if constrained {
                        continue;
                    }
                    let [p, q] = pair(i);
                    let (p, q) = (self.plane(p.0), self.plane(q.0));
                    let length = (p.0 - q.0).hypot(p.1 - q.1);
                    if longest.is_none_or(|(l, _)| length > l) {
                        longest = Some((length, i));
                    }
                }
                return match longest {
                    Some((_, i)) => Action::SplitEdge(i),
                    None => Action::SplitTriangle,
                };
            }
        }
        if let Some(angle) = self.angle {
            // The free side over which the surface's normal turns most,
            // of those that turn it further than the angle and are not
            // below the floor. A side on the boundary is a chord of an
            // edge, which the edge holds to the angle by its tangent and
            // which no point of the face can change; a corner without a
            // normal (an apex) tells nothing.
            let normals = view.v.map(|v| self.normal_of(v));
            let mut worst: Option<(f64, usize)> = None;
            for (i, constrained) in [c0, c1, c2].into_iter().enumerate() {
                if constrained {
                    continue;
                }
                let (j, k) = match i {
                    0 => (1, 2),
                    1 => (2, 0),
                    _ => (0, 1),
                };
                let (Some(Some(x)), Some(Some(y))) = (normals.get(j), normals.get(k)) else {
                    continue;
                };
                let [p, q] = pair(i);
                let Some(turn) = crate::angle_between(*x, *y) else {
                    continue;
                };
                if crate::turns_beyond(turn, angle)
                    && p.1.distance(q.1) >= self.angle_floor
                    && worst.is_none_or(|(w, _)| turn > w)
                {
                    worst = Some((turn, i));
                }
            }
            if let Some((_, i)) = worst {
                return Action::Turn(i);
            }
        }
        Action::Keep
    }

    /// Adds a point in the middle of edge `i` of triangle `t`.
    fn split_edge(&mut self, t: usize, i: usize) -> Result<(), CdtError> {
        let view = self.cdt.triangle(t).ok_or(CdtError::Internal)?;
        let [x, y, z] = view.v;
        let (p, q) = match i % 3 {
            0 => (y, z),
            1 => (z, x),
            _ => (x, y),
        };
        let (Some(p), Some(q)) = (self.vertex(p), self.vertex(q)) else {
            return Err(CdtError::Internal);
        };
        let ([pa, pb], [qa, qb]) = (p.0, q.0);
        let uv = [0.5 * (pa + qa), 0.5 * (pb + qb)];
        let v = self.cdt.split_edge(t as u32, i, self.plane(uv))?;
        self.record(v, uv, self.param.eval(uv), None)
    }

    /// Adds a point at the centroid of triangle `t`.
    fn split_triangle(&mut self, t: usize) -> Result<(), CdtError> {
        let view = self.cdt.triangle(t).ok_or(CdtError::Internal)?;
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| self.vertex(v)) else {
            return Err(CdtError::Internal);
        };
        let ([a0, a1], [b0, b1], [c0, c1]) = (a.0, b.0, c.0);
        let uv = [(a0 + b0 + c0) / 3.0, (a1 + b1 + c1) / 3.0];
        let v = self.cdt.split_triangle(t as u32, self.plane(uv))?;
        self.record(v, uv, self.param.eval(uv), None)
    }
}

/// Cuts every stretch of the boundary that is one point in space (two
/// neighbouring pole vertices) into grid-sized steps, and gives every seam
/// its points: the same 3D points on both sides of the cut.
fn finish_boundary(
    param: &Param,
    region: &mut Region,
    step: Uv,
    tolerance: f64,
    limit: usize,
    face: usize,
) -> Result<(), TessError> {
    let too_many = || TessError::TooFine {
        patch: face,
        needed_segments: limit as f64,
        max_segments: 0,
    };
    let old: Vec<(usize, usize)> = std::mem::take(&mut region.segs);
    let old_chords: Vec<Option<Chord>> = std::mem::take(&mut region.chords);
    for (n, (i, j)) in old.into_iter().enumerate() {
        let chord = old_chords.get(n).copied().flatten();
        let (Some(&p), Some(&q)) = (region.verts.get(i), region.verts.get(j)) else {
            continue;
        };
        let (pieces, pole) = match (p.free, q.free) {
            (Some(k), Some(l)) if k == l && p.pos == q.pos => {
                let length = (get2(q.uv, k) - get2(p.uv, k)).abs();
                ((length / get2(step, k) - 1e-9).ceil(), true)
            }
            // Into or out of the corner of a pole at a corner of the
            // parameters (`CornerPole`): along whichever parameter moves.
            (Some(_), Some(_)) if p.pos == q.pos => {
                let length = |k: usize| (get2(q.uv, k) - get2(p.uv, k)).abs() / get2(step, k);
                ((length(0).max(length(1)) - 1e-9).ceil(), true)
            }
            _ => (1.0, false),
        };
        if exceeds(pieces, limit as f64) {
            return Err(too_many());
        }
        // In range by the check above.
        let pieces = (pieces as usize).max(1);
        let mut last = i;
        for n in 1..pieces {
            let s = n as f64 / pieces as f64;
            let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
            let next = region.add(Bv {
                uv: [pa + (qa - pa) * s, pb + (qb - pb) * s],
                chord: None,
                ..p
            });
            region.link(last, next, None);
            last = next;
        }
        // A stretch of a pole is one point in space, not a chord.
        region.link(last, j, if pole { None } else { chord });
    }
    for seam in region.seams.clone() {
        let (Some(&from), Some(&to)) = (region.verts.get(seam.from), region.verts.get(seam.to))
        else {
            continue;
        };
        let ([fa, fb], [ta, tb]) = (from.uv, to.uv);
        let [sa, sb] = step;
        let pieces = ((ta - fa).abs() / sa).max((tb - fb).abs() / sb);
        let pieces = (pieces - 1e-9).ceil();
        if exceeds(pieces, limit as f64) {
            return Err(too_many());
        }
        // In range by the check above.
        let pieces = (pieces as usize).max(1);
        let at = |s: f64| -> (f64, Uv, Vec3) {
            let uv = [fa + (ta - fa) * s, fb + (tb - fb) * s];
            (s, uv, param.eval(uv))
        };
        let mut line: Vec<(f64, Uv, Vec3)> = vec![(0.0, from.uv, from.pos)];
        line.extend((1..pieces).map(|n| at(n as f64 / pieces as f64)));
        line.push((1.0, to.uv, to.pos));
        // Halve the pieces whose chord is not within the tolerance of the
        // seam curve. Half the tolerance, as for the grid: a seam mostly
        // runs across a doubly curved surface.
        if !param.is_plane() {
            for _ in 0..24 {
                let mut refined = Vec::with_capacity(line.len());
                let mut changed = false;
                for pair in line.windows(2) {
                    let [p, q] = pair else { continue };
                    refined.push(*p);
                    let middle = at(0.5 * (p.0 + q.0));
                    if middle.2.distance(p.2.lerp(q.2, 0.5)) > tolerance / 2.0 {
                        refined.push(middle);
                        changed = true;
                    }
                }
                if let Some(last) = line.last() {
                    refined.push(*last);
                }
                line = refined;
                if line.len() > limit {
                    return Err(too_many());
                }
                if !changed {
                    break;
                }
            }
        }
        // Halve the pieces that a vertex of the region lies beside on the
        // other side in space than in the plane (`passed_on_the_wrong_side`):
        // a seam that leaves a vertex almost along a chord of the loop
        // there, or runs past a vertex of a loop closer than its pieces'
        // sag. The refinement cannot cut a seam, so it is cut here, as an
        // edge's chord is cut when a face asks for it.
        for _ in 0..SEAM_HALVINGS {
            let crossed: Vec<usize> = line
                .windows(2)
                .enumerate()
                .filter(|(_, pair)| {
                    let [p, q] = pair else { return false };
                    passed_on_the_wrong_side(param, region, (p.1, p.2), (q.1, q.2), seam.shift)
                })
                .map(|(k, _)| k)
                .collect();
            if crossed.is_empty() {
                break;
            }
            for &k in crossed.iter().rev() {
                let (Some(p), Some(q)) = (line.get(k), line.get(k + 1)) else {
                    continue;
                };
                let middle = at(0.5 * (p.0 + q.0));
                line.insert(k + 1, middle);
            }
            if line.len() > limit {
                return Err(too_many());
            }
        }
        let (mut last, mut twin_last) = (seam.from, seam.twin_from);
        for &(_, uv, pos) in line.iter().skip(1).take(line.len().saturating_sub(2)) {
            let ([a, b], [da, db]) = (uv, seam.shift);
            let next = region.add(Bv {
                uv,
                pos,
                free: None,
                chord: None,
            });
            let twin_next = region.add(Bv {
                uv: [a + da, b + db],
                pos,
                free: None,
                chord: None,
            });
            region.link(last, next, None);
            region.link(twin_last, twin_next, None);
            (last, twin_last) = (next, twin_next);
        }
        region.link(last, seam.to, None);
        region.link(twin_last, seam.twin_to, None);
    }
    Ok(())
}

/// How many rounds of halving the seam's pieces get at most because a
/// vertex of the region lies beside one on the other side in space than
/// in the plane (`passed_on_the_wrong_side`): a piece 2⁻²⁴ of what it was
/// is as near its curve as a seam gets; a vertex still on the wrong side
/// then lies on the seam's curve, and the refinement says so.
const SEAM_HALVINGS: usize = 24;

/// Whether a vertex of the region lies, in space, on the other side of
/// the seam piece from `p` to `q` than in the parameter plane, among those
/// within the piece's diametral circle on the surface (measured in the
/// tangent plane at the piece's middle, through the derivatives there:
/// the vertices a triangulation joins to the piece's ends in a triangle
/// that stands on it, as in Ruppert's encroachment). The piece is taken in
/// its copy (as given, or moved by `shift`) nearer to the vertex. A
/// triangle between the piece and such a vertex faces, in space, against
/// the surface, and no point the refinement adds mends it, since the
/// piece is a side it may not cut. Cut in two, the piece's chords lie
/// four times nearer to the seam's curve and their circles are half as
/// wide, so a vertex that is not on the seam's curve itself is out of
/// them or on its own side after a few halvings. (At a pole, where the
/// derivatives have no cross product, nothing can be told: false.)
fn passed_on_the_wrong_side(
    param: &Param,
    region: &Region,
    (p_uv, p): (Uv, Vec3),
    (q_uv, q): (Uv, Vec3),
    shift: Uv,
) -> bool {
    let middle = [0.5 * (p_uv[0] + q_uv[0]), 0.5 * (p_uv[1] + q_uv[1])];
    let (_, [da, db]) = param.derivatives(middle);
    let reference = da.cross(db);
    let along = |x: f64, y: f64| da * x + db * y;
    let radius = 0.5 * along(q_uv[0] - p_uv[0], q_uv[1] - p_uv[1]).length();
    if !positive(reference.length_squared()) || !positive(radius) {
        return false;
    }
    let chord = q - p;
    let twin = [middle[0] + shift[0], middle[1] + shift[1]];
    region.verts.iter().any(|v| {
        if v.free.is_some() || v.pos == p || v.pos == q {
            return false;
        }
        // From the middle of the piece, in whichever copy is nearer.
        let from = |m: Uv| along(v.uv[0] - m[0], v.uv[1] - m[1]).length();
        let (near, far) = (from(middle), from(twin));
        let moved = if far < near { shift } else { [0.0, 0.0] };
        if near.min(far) >= radius {
            return false;
        }
        let (pa, pb) = (p_uv[0] + moved[0], p_uv[1] + moved[1]);
        let (qa, qb) = (q_uv[0] + moved[0], q_uv[1] + moved[1]);
        let plane = (qa - pa) * (v.uv[1] - pb) - (qb - pb) * (v.uv[0] - pa);
        let space = chord.cross(v.pos - p).dot(reference);
        (plane < 0.0 && space > 0.0) || (plane > 0.0 && space < 0.0)
    })
}

/// How many pieces each parameter of a spline face is cut into to see
/// where its normal turns ([`angle_warp`]), before pieces that turn
/// further than a cell may are cut again.
const WARP_PIECES: usize = 32;

/// How many lines across the face each piece is looked at along.
const WARP_ACROSS: usize = 9;

/// How many pieces are looked at per parameter at most.
const WARP_MOST: usize = 1024;

/// A monotone map of one parameter onto one axis of the plane a spline
/// face is triangulated in (with the angle criterion, [`angle_warp`]):
/// piecewise linear through `(at, level)`, `level` increasing with `at`,
/// one unit per cell of the face's grid, zero at the centre of the face.
#[derive(Clone, Debug)]
struct Warp {
    at: Vec<f64>,
    level: Vec<f64>,
}

impl Warp {
    /// The level of `x`; beyond the ends, along the first or last piece.
    fn map(&self, x: f64) -> f64 {
        let n = self.at.partition_point(|&a| a <= x);
        let k = n.clamp(1, self.at.len().saturating_sub(1).max(1));
        let (Some(&a0), Some(&a1), Some(&l0), Some(&l1)) = (
            self.at.get(k - 1),
            self.at.get(k),
            self.level.get(k - 1),
            self.level.get(k),
        ) else {
            return 0.0;
        };
        if a1 > a0 {
            l0 + (l1 - l0) * (x - a0) / (a1 - a0)
        } else {
            l0
        }
    }

    /// The parameter at `level`, within the ends.
    fn unmap(&self, level: f64) -> Option<f64> {
        let n = self.level.partition_point(|&l| l <= level);
        let k = n.clamp(1, self.level.len().saturating_sub(1).max(1));
        let (a0, a1, l0, l1) = (
            *self.at.get(k - 1)?,
            *self.at.get(k)?,
            *self.level.get(k - 1)?,
            *self.level.get(k)?,
        );
        Some(if l1 > l0 {
            a0 + (a1 - a0) * ((level - l0) / (l1 - l0)).clamp(0.0, 1.0)
        } else {
            a0
        })
    }
}

/// With the angle criterion, the plane a spline face is triangulated in
/// and the grid it is seeded with, where the surface turns its normal
/// unevenly; `None` where the chordal grid (`cells` over `low..high`)
/// meets the angle everywhere already, where the surface has no normal
/// at a sample, or where the grid would need more than `max_segments`
/// cells a parameter.
///
/// A spline face can turn its normal very unevenly, and its parameters
/// can run very unevenly over it: a side of an aerofoil turns half a turn
/// around its nose in a few hundredths of its chord and hardly at all
/// along its span, and the parameter across it moves forty times slower
/// near the nose than at the root of the side (the turbofan's fan
/// blades). One scale per parameter (lengths in space, as everywhere
/// else, or the cells of the grid, as on a torus) fits one part of such
/// a face and not the rest; and the uniform grid of the chordal tolerance
/// knows nothing of the angle, so the refinement put the points the
/// angle asks for one by one, each joined by Delaunay's rule to points
/// where the plane said they were near and the surface did not (1.9
/// times the triangles of a grid that meets both criteria, on the
/// twisted blade of `crates/testkit/tests/spline_angle.rs`).
///
/// So each parameter is cut into [`WARP_PIECES`] pieces, the normal is
/// sampled at the ends and the middle of each along [`WARP_ACROSS`] lines
/// across the face's box, and a piece over which it turns further than
/// a cell may (`angle / √2`, so that a cell's diagonal keeps within the
/// angle) is cut in two and looked at again. Each piece gets as many
/// cells as its turn asks for, and at least the chordal grid's share;
/// where the sides across the face are below the floor (`ANGLE_FLOOR`
/// tolerances), none shorter than the floor along it either, so that the
/// angle never asks for more than a fine tolerance would (a count of
/// ratios, no length of its own; where the sides across are longer, their
/// diagonals are held to the angle, and the pieces get the cells the
/// angle asks for whatever the floor, as a thin torus does). The plane
/// along the parameter is then the count of cells so far: a cell of the
/// grid is a unit square of the plane everywhere, the grid is graded to
/// where the surface turns, and a point added stays among its
/// neighbours. A sample, not a bound: the refinement still holds every
/// side to the angle; the grid puts the points where it would have.
fn angle_warp(
    param: &Param,
    low: Uv,
    high: Uv,
    centre: Uv,
    cells: [u32; 2],
    params: &TessParams,
) -> Option<([Warp; 2], [Vec<f64>; 2])> {
    let angle = params.max_angle?;
    let per_cell = angle * std::f64::consts::FRAC_1_SQRT_2;
    let floor = crate::ANGLE_FLOOR * params.chordal_tolerance;
    let most = f64::from(params.max_segments);
    let mut found: Vec<(Warp, Vec<f64>)> = Vec::with_capacity(2);
    let mut finer = false;
    for k in 0..2 {
        let (from, to) = (get2(low, k), get2(high, k));
        let span = to - from;
        let other = 1 - k;
        let (across_from, across_to) = (get2(low, other), get2(high, other));
        let uniform = f64::from(get2(cells, k).max(1));
        if !(span.is_finite() && span > 0.0) {
            return None;
        }
        let at = |along: f64, across: f64| -> Uv {
            if k == 0 {
                [along, across]
            } else {
                [across, along]
            }
        };
        // The surface's point and normal along each line across, at `x`.
        let sample = |x: f64| -> Option<Vec<(Vec3, Vec3)>> {
            (0..WARP_ACROSS)
                .map(|line| {
                    let across = across_from
                        + (across_to - across_from) * (line as f64) / ((WARP_ACROSS - 1) as f64);
                    let uv = at(x, across);
                    Some((param.eval(uv), param.normal(uv)?))
                })
                .collect()
        };
        // How long a side of a cell across the face is, at most.
        let mut across_side = 0.0_f64;
        let mut ends = Vec::with_capacity(WARP_PIECES + 1);
        for n in 0..=WARP_PIECES {
            let x = from + span * (n as f64) / (WARP_PIECES as f64);
            let line = sample(x)?;
            let length: f64 = line
                .windows(2)
                .map(|w| match w {
                    [(p, _), (q, _)] => p.distance(*q),
                    _ => 0.0,
                })
                .sum();
            across_side = across_side.max(length / f64::from(get2(cells, other).max(1)));
            ends.push((x, line));
        }
        let angle_alone = across_side >= floor;
        // Pieces still to look at, the leftmost on top; then per piece,
        // from left to right, its start, width and cells per unit of the
        // parameter.
        let mut pieces = Vec::with_capacity(WARP_PIECES);
        for pair in ends.windows(2).rev() {
            if let [(a, sa), (b, sb)] = pair {
                pieces.push((*a, *b, sa.clone(), sb.clone()));
            }
        }
        let mut density: Vec<(f64, f64, f64)> = Vec::new();
        let mut looked_at = 0_usize;
        while let Some((a, b, sa, sb)) = pieces.pop() {
            looked_at += 1;
            let m = 0.5 * (a + b);
            let sm = sample(m)?;
            let (mut turn, mut length) = (0.0_f64, 0.0_f64);
            for ((pa, na), ((pm, nm), (pb, nb))) in sa.iter().zip(sm.iter().zip(&sb)) {
                let here = crate::angle_between(*na, *nm)? + crate::angle_between(*nm, *nb)?;
                turn = turn.max(here);
                length = length.max(pa.distance(*pm) + pm.distance(*pb));
            }
            let mut by_angle = turn / per_cell;
            if !angle_alone {
                by_angle = by_angle.min(length / floor);
            }
            if by_angle > 1.0 && a < m && m < b && looked_at + pieces.len() < WARP_MOST {
                pieces.push((m, b, sm.clone(), sb));
                pieces.push((a, m, sa, sm));
                continue;
            }
            let width = b - a;
            let wanted = by_angle / width;
            if !(wanted.is_finite() && width > 0.0) {
                return None;
            }
            finer |= wanted > uniform / span * (1.0 + 1e-9);
            density.push((a, width, wanted.max(uniform / span)));
        }
        let total: f64 = density.iter().map(|&(_, w, d)| w * d).sum();
        let count = (total - 1e-9).ceil().max(uniform);
        if !(count <= most && total > 0.0) {
            return None;
        }
        // One unit of the plane per cell: the measure times `count / total`.
        let unit = count / total;
        let mut warp = Warp {
            at: vec![from],
            level: vec![0.0],
        };
        let mut reached = 0.0;
        for &(a, width, d) in &density {
            reached += width * d * unit;
            warp.at.push(a + width);
            warp.level.push(reached);
        }
        if let Some(last) = warp.at.last_mut() {
            *last = to;
        }
        let lines: Vec<f64> = (0..=(count as usize))
            .filter_map(|i| warp.unmap(i as f64))
            .collect();
        let middle = warp.map(get2(centre, k));
        for level in &mut warp.level {
            *level -= middle;
        }
        found.push((warp, lines));
    }
    if !finer {
        return None;
    }
    let [(wa, la), (wb, lb)]: [(Warp, Vec<f64>); 2] = found.try_into().ok()?;
    Some(([wa, wb], [la, lb]))
}

/// The grid nodes `(i, j)`, `0 < i < columns`, `0 < j < rows`, coarse
/// levels first: points arrive spread out, as a Delaunay triangulation
/// likes them, and each next to one that is already there.
fn grid_order(columns: u32, rows: u32) -> Vec<(u32, u32)> {
    let mut order = Vec::new();
    let mut stride = columns.max(rows).next_power_of_two();
    let top = stride;
    while stride >= 1 {
        let mut reverse = false;
        let mut j = stride;
        while j < rows {
            let mut row = Vec::new();
            let mut i = stride;
            while i < columns {
                let coarser =
                    stride < top && i.is_multiple_of(2 * stride) && j.is_multiple_of(2 * stride);
                if !coarser {
                    row.push((i, j));
                }
                i += stride;
            }
            if reverse {
                row.reverse();
            }
            reverse = !reverse;
            order.extend(row);
            j += stride;
        }
        stride /= 2;
    }
    order
}

/// The boundary of a face, ready to be triangulated: its region in
/// parameter space, and the grid and the scales that go with it.
pub(crate) struct Boundary<'a> {
    param: Param<'a>,
    region: Region,
    low: Uv,
    high: Uv,
    cells: [u32; 2],
    step: Uv,
    /// With the angle criterion on a spline face whose normal turns
    /// unevenly: the plane it is triangulated in, per parameter, and the
    /// lines of its grid ([`angle_warp`]). Tried first; the face is
    /// triangulated as without them where that fails.
    warp: Option<([Warp; 2], [Vec<f64>; 2])>,
    scale: Uv,
    /// How much further than lengths in space the plane is scaled, per
    /// parameter: 1 but where it is the plane of the grid (see
    /// `face_boundary`).
    stretch: Uv,
    centre: Uv,
    /// No face may grow beyond this many vertices.
    limit: usize,
}

/// A point of parameter space in the plane the triangulation works in.
fn to_plane(uv: Uv, centre: Uv, scale: Uv) -> P2 {
    let ([a, b], [ca, cb], [sa, sb]) = (uv, centre, scale);
    ((a - ca) * sa, (b - cb) * sb)
}

/// The part of its longest side within which a triangle that cannot be
/// refined lies flat against a chord of the boundary (`mesh_face`).
const FLAT_AGAINST: f64 = 1e-6;

/// How many of the triangles still failing after the last round of
/// refinement are looked at for the chords they stand on (`mesh_face`).
const STUCK_LOOKED_AT: usize = 64;

/// The distance from `p` to the segment from `a` to `b`, in the plane.
fn plane_distance_to_segment(p: P2, a: P2, b: P2) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let t = if t.is_nan() { 0.0 } else { t };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// The distance from `p` to the segment from `a` to `b`.
fn distance_to_chord(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    p.distance(nearest_on_chord(p, a, b))
}

/// The point of the segment from `a` to `b` nearest to `p`.
fn nearest_on_chord(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let along = b - a;
    let whole = along.length_squared();
    let s = if positive(whole) {
        ((p - a).dot(along) / whole).clamp(0.0, 1.0)
    } else {
        0.0
    };
    a + along * s
}

/// How far the surface over the middle of the chord from `p` to `q` (in
/// parameter space) lies beside the chord, along the surface: its bulge,
/// when it is more than rounding (`chords_to_refine`).
fn chord_bulge(param: &Param, p: &Bv, q: &Bv) -> Option<f64> {
    let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
    let middle = [0.5 * (pa + qa), 0.5 * (pb + qb)];
    let image = param.eval(middle);
    let (_, [da, db]) = param.derivatives(middle);
    let normal = da.cross(db).normalized()?;
    let beside = image - nearest_on_chord(image, p.pos, q.pos);
    let aside = (beside - normal * beside.dot(normal)).length();
    (aside.is_finite() && aside > SIDE_NOISE * p.pos.distance(q.pos)).then_some(aside)
}

/// How many times its own bulge (how far the surface over the middle of a
/// chord lies beside the chord, along the surface) another part of the
/// boundary must keep away from the chord, along the surface, for the
/// chord to do.
const NEAR_BULGE: f64 = 4.0;
/// The part of a chord's length below which a bulge is rounding.
const SIDE_NOISE: f64 = 1e-9;

impl Boundary<'_> {
    /// The chords of edges that this face needs cut in two, because the
    /// edges were turned into points without regard to it (`edge.rs`):
    ///
    /// - chords that cross or touch another segment of the boundary. The
    ///   face's edges do not cross, so chords fine enough do not either;
    ///   but an edge that comes nearer to another than the chordal
    ///   tolerance may have a chord that cuts the corner (a hole close to
    ///   a rim, a notch that stops just short of an arc).
    /// - chords that are not what the triangulation takes them for. It
    ///   works in parameter space, where the chord is the straight segment
    ///   between its ends; the surface along that segment must stay near
    ///   the chord. Close to a pole it does not: the segment runs along a
    ///   parallel where the chord cuts across. The limit is twice the
    ///   chordal tolerance: one for the chord's own sag from its curve,
    ///   and one more that the segment's image may be from the curve.
    /// - chords on a curved surface that another part of the boundary
    ///   comes nearer to, along the surface, than [`NEAR_BULGE`] times
    ///   what the chord cuts off beside it: across a strip narrower than
    ///   that the chord passes the other side of the strip, the first
    ///   case again, where the parameters show nothing of it.
    ///
    /// In no particular order, and a chord may be named more than once.
    /// Empty for a boundary that is fine, and for what finer chords cannot
    /// cure: straight edges on a surface whose straight lines are
    /// straight in its parameters, seams, poles.
    pub(crate) fn chords_to_refine(&self, body: &Body, params: &TessParams) -> Vec<Chord> {
        let tolerance = params.chordal_tolerance;
        let region = &self.region;
        let analytic = !self.param.is_freeform();
        // A straight edge on a plane, a cylinder or a cone is its own
        // chord, in space and in parameters: cutting it changes nothing.
        let curable = |chord: Option<Chord>| -> Option<Chord> {
            let (edge, _) = chord?;
            let curve = &body.edges.get(edge as usize)?.curve;
            (!(analytic && matches!(curve, Curve3::Line(_)))).then_some(chord?)
        };
        struct Segment {
            low: P2,
            high: P2,
            ends: [P2; 2],
            chord: Option<Chord>,
            /// In space: the ends, and for a chord on a curved surface
            /// what it cuts off.
            pos: [Vec3; 2],
            bulge: Option<Bulge>,
        }
        /// The surface point over the middle of a chord's segment in
        /// parameter space, the unit normal there, and how far the point
        /// lies beside the chord, measured along the surface.
        #[derive(Clone, Copy)]
        struct Bulge {
            image: Vec3,
            normal: Vec3,
            aside: f64,
        }
        let mut segments: Vec<Segment> = Vec::with_capacity(region.segs.len());
        let mut wanted: Vec<Chord> = Vec::new();
        let mut any_curable = false;
        for (n, &(i, j)) in region.segs.iter().enumerate() {
            let (Some(p), Some(q)) = (region.verts.get(i), region.verts.get(j)) else {
                continue;
            };
            let chord = curable(region.chords.get(n).copied().flatten());
            any_curable |= chord.is_some();
            let mut bulge = None;
            if let Some(chord) = chord
                && !self.param.is_plane()
            {
                let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
                let middle = [0.5 * (pa + qa), 0.5 * (pb + qb)];
                let image = self.param.eval(middle);
                let off = distance_to_chord(image, p.pos, q.pos);
                if exceeds(off, 2.0 * tolerance) {
                    wanted.push(chord);
                } else if let Some(angle) = params.angle_for(p.pos.distance(q.pos)) {
                    // The angle criterion along the face: the edge holds
                    // its own tangent to it, but the surface's normal may
                    // turn further along the chord than the tangent does
                    // (an ellipse cut obliquely from a cylinder).
                    let turn = self
                        .param
                        .normal(p.uv)
                        .zip(self.param.normal(q.uv))
                        .and_then(|(x, y)| crate::angle_between(x, y));
                    if turn.is_some_and(|turn| crate::turns_beyond(turn, angle)) {
                        wanted.push(chord);
                    }
                }
                // How far the surface there lies beside the chord, along
                // the surface: a chord of a rim is off a cylinder by its
                // sag, towards the axis, and beside nothing; one that
                // cuts the corner of a curve in the surface is beside it.
                let (_, [da, db]) = self.param.derivatives(middle);
                if let Some(normal) = da.cross(db).normalized() {
                    let along = |v: Vec3| (v - normal * v.dot(normal)).length();
                    let aside = along(image - nearest_on_chord(image, p.pos, q.pos));
                    if aside.is_finite() && aside > SIDE_NOISE * p.pos.distance(q.pos) {
                        bulge = Some(Bulge {
                            image,
                            normal,
                            aside,
                        });
                    }
                }
            }
            let a = to_plane(p.uv, self.centre, self.scale);
            let b = to_plane(q.uv, self.centre, self.scale);
            // (The plane is scaled to lengths in space, or `stretch` times
            // that, so the reach of a bulge there is about its reach in
            // space, times `stretch`.)
            let reach = bulge.map_or(0.0, |bulge| NEAR_BULGE * bulge.aside);
            let [ra, rb] = self.stretch.map(|k| k * reach);
            segments.push(Segment {
                low: (a.0.min(b.0) - ra, a.1.min(b.1) - rb),
                high: (a.0.max(b.0) + ra, a.1.max(b.1) + rb),
                ends: [a, b],
                chord,
                pos: [p.pos, q.pos],
                bulge,
            });
        }
        if let (true, Some(([wa, wb], _))) = (any_curable, &self.warp) {
            // The same crossings in the plane of the warp, which the face
            // is triangulated in first.
            let mut warped: Vec<([P2; 2], Option<Chord>)> = region
                .segs
                .iter()
                .enumerate()
                .filter_map(|(n, &(i, j))| {
                    let (p, q) = (region.verts.get(i)?, region.verts.get(j)?);
                    let plane = |uv: Uv| (wa.map(uv[0]), wb.map(uv[1]));
                    let chord = curable(region.chords.get(n).copied().flatten());
                    Some(([plane(p.uv), plane(q.uv)], chord))
                })
                .collect();
            warped.sort_by(|x, y| x.0[0].0.min(x.0[1].0).total_cmp(&y.0[0].0.min(y.0[1].0)));
            for (n, ([a, b], first)) in warped.iter().enumerate() {
                let right = a.0.max(b.0);
                for ([c, d], second) in warped.iter().skip(n + 1) {
                    if c.0.min(d.0) > right {
                        break;
                    }
                    if first.is_none() && second.is_none() {
                        continue;
                    }
                    if seam_blocked([a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]) {
                        wanted.extend(*first);
                        wanted.extend(*second);
                    }
                }
            }
        }
        if any_curable {
            // Every pair of segments whose boxes overlap, found by going
            // through them from left to right.
            segments.sort_by(|x, y| x.low.0.total_cmp(&y.low.0));
            for (n, first) in segments.iter().enumerate() {
                for second in segments.iter().skip(n + 1) {
                    if second.low.0 > first.high.0 {
                        break;
                    }
                    if (first.chord.is_none() && second.chord.is_none())
                        || second.low.1 > first.high.1
                        || first.low.1 > second.high.1
                    {
                        continue;
                    }
                    // (Neighbours along a loop, and loops that touch in a
                    // vertex, meet end to end: that is no crossing.)
                    let ([a, b], [c, d]) = (first.ends, second.ends);
                    if seam_blocked([a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]) {
                        wanted.extend(first.chord);
                        wanted.extend(second.chord);
                    }
                    // The same in space. A chord that cuts across a strip
                    // narrower than its own bulge passes the other side
                    // of the strip, and the triangles between the two
                    // face backwards, however the two lie in parameter
                    // space (straight and apart, on a strip along a
                    // border of a patch). Measured along the surface:
                    // what lies beside the chord there, not over it.
                    let apart = [a, b].iter().all(|end| !c.eq(end) && !d.eq(end));
                    if apart {
                        for (one, other) in [(first, second), (second, first)] {
                            let (Some(bulge), [from, to]) = (one.bulge, other.pos) else {
                                continue;
                            };
                            let across = nearest_on_chord(bulge.image, from, to) - bulge.image;
                            let beside =
                                (across - bulge.normal * across.dot(bulge.normal)).length();
                            if beside < NEAR_BULGE * bulge.aside {
                                wanted.extend(one.chord);
                            }
                        }
                    }
                }
            }
        }
        wanted
    }
}

/// Every chord of the edges of `face` that cutting in two can change: all
/// of them but those of straight edges on a surface whose straight lines
/// are straight in its parameters. What a face asks for whose loops, as
/// polygons, enclose nothing (see [`face_boundary`]).
pub(crate) fn curved_chords(body: &Body, edges: &[Vec<Vec3>], face: &Face) -> Vec<Chord> {
    let freeform = matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_));
    let mut chords = Vec::new();
    let coedges = face
        .loops
        .iter()
        .filter_map(|id| body.loop_(*id))
        .flat_map(|loop_| &loop_.coedges);
    for coedge in coedges {
        let id = coedge.edge.0;
        let (Some(edge), Some(points)) = (body.edge(coedge.edge), edges.get(id as usize)) else {
            continue;
        };
        if !freeform && matches!(edge.curve, Curve3::Line(_)) {
            continue;
        }
        let count = u32::try_from(points.len().saturating_sub(1)).unwrap_or(u32::MAX);
        chords.extend((0..count).map(|chord| (id, chord)));
    }
    chords
}

/// The boundary of face `index` of `body`, whose edges are already points.
///
/// `starved` is set when the face is refused because its loops, as the
/// polygons the edge points make of them, enclose no area: a face
/// narrower than the chordal tolerance between a curved edge and its
/// chord (a sliver between a line and a flat arc from the same vertex to
/// the same vertex is, at one chord for the arc, a line there and back).
/// That says nothing about the face, only about the points: with the
/// chords of its curved edges cut in two ([`curved_chords`]) it may well
/// enclose some.
pub(crate) fn face_boundary<'a>(
    body: &Body,
    edges: &[Vec<Vec3>],
    face: &'a Face,
    index: usize,
    params: &TessParams,
    starved: &mut bool,
) -> Result<Boundary<'a>, TessError> {
    let loops = loop_points(body, edges, face, index)?;
    let tolerance = params.chordal_tolerance;

    // The loops in parameter space, with the parameters swapped when they
    // wind around the second one.
    let mut found: Option<(Param, Vec<Ring>, f64)> = None;
    let closure = Closure::of(&face.surface);
    for swapped in [false, true] {
        let param = Param {
            surface: &face.surface,
            swapped,
            closure,
        };
        // Loops run counter-clockwise around the outward normal, which is
        // the surface's `∂u × ∂v` unless the face is reversed.
        let sense = if swapped != face.reversed { -1.0 } else { 1.0 };
        let mut rings = Vec::with_capacity(loops.len());
        for (number, points) in loops.iter().enumerate() {
            rings.push(build_ring(&param, points, sense, index, number)?);
        }
        let around = |k: usize| rings.iter().any(|r| get2(r.winding, k) != 0);
        if rings.iter().any(|r| r.winding.iter().any(|w| w.abs() > 1)) {
            return unsupported(index, "a loop that winds around its surface more than once");
        }
        match (around(0), around(1)) {
            (true, true) => {
                return unsupported(
                    index,
                    "loops that wind around both the axis and the tube of a torus",
                );
            }
            (false, true) if !swapped => continue,
            _ => {
                found = Some((param, rings, sense));
                break;
            }
        }
    }
    let Some((param, rings, sense)) = found else {
        return unsupported(
            index,
            "loops that wind around its surface in a way not handled",
        );
    };
    let winding = rings.iter().filter(|r| r.winding != [0, 0]).count();
    // Loops that all run clockwise bound nothing in a plane. On a closed
    // surface they bound the rest of it: the face is the whole surface
    // with those loops as holes (the zone left of a ball with a hole
    // drilled through it across the sphere's axis).
    let periods = param.periods();
    let around_holes = winding == 0
        && !rings.is_empty()
        && rings
            .iter()
            .all(|r| positive(-r.twice_area(periods) * sense))
        && closes_to_points(&param);
    let mut region = if rings.is_empty() || winding > 0 || around_holes {
        if winding > 2 {
            return fail(
                index,
                "its loops do not bound a region: more than two of them wind around the axis",
            );
        }
        band_region(&param, &rings, sense, index, starved)?
    } else {
        patch_region(&param, &rings, sense, index, starved)?
    };

    // The box the region lies in, and the grid over it.
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for v in &region.verts {
        for k in 0..2 {
            let x = get2(v.uv, k);
            if !x.is_finite() {
                return fail(index, "its boundary has no finite surface parameters");
            }
            let (lowest, highest) = (get2(low, k).min(x), get2(high, k).max(x));
            set2(&mut low, k, lowest);
            set2(&mut high, k, highest);
        }
    }
    let span = [get2(high, 0) - get2(low, 0), get2(high, 1) - get2(low, 1)];
    // On a spline surface the parameters of a point are found by a search,
    // to rounding: a boundary that runs there and back along one line (the
    // seam of a whole closed surface, given as an edge) encloses a sliver
    // of that rounding, not nothing. Narrower than this part of the
    // surface's own range, a region is no region.
    let least = |k: usize| -> f64 {
        let (from, to) = get2(param.domain(), k);
        if param.is_freeform() && from.is_finite() && to.is_finite() {
            NO_REGION * (to - from)
        } else {
            0.0
        }
    };
    if !(0..2).all(|k| get2(span, k).is_finite() && get2(span, k) > least(k)) {
        *starved = true;
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }
    let cells = if param.is_plane() {
        [1, 1]
    } else {
        let (low, high) = (param.raw(low), param.raw(high));
        let (nu, nv) = seed_cells(&face.surface, &param.closure, low, high, params, index)?;
        param.order(nu, nv)
    };
    let step = [
        get2(span, 0) / f64::from(get2(cells, 0)),
        get2(span, 1) / f64::from(get2(cells, 1)),
    ];
    // No face may grow beyond this many vertices.
    let side = (params.max_segments as usize).saturating_add(1);
    let limit = side.saturating_mul(side).saturating_mul(4);
    finish_boundary(&param, &mut region, step, tolerance, limit, index)?;
    if region.verts.len() > limit {
        return Err(TessError::TooFine {
            patch: index,
            needed_segments: (region.verts.len() as f64).sqrt(),
            max_segments: params.max_segments,
        });
    }

    // Scale the parameters by how fast the surface moves with them, so
    // that "Delaunay" means roughly what it would in space.
    let mut scale = [1.0_f64; 2];
    if !param.is_plane() {
        let mut largest = [0.0_f64; 2];
        for i in 0..5 {
            for j in 0..5 {
                let uv = [
                    get2(low, 0) + get2(span, 0) * f64::from(i) / 4.0,
                    get2(low, 1) + get2(span, 1) * f64::from(j) / 4.0,
                ];
                let (_, [da, db]) = param.derivatives(uv);
                let [la, lb] = &mut largest;
                *la = la.max(da.length());
                *lb = lb.max(db.length());
            }
        }
        for k in 0..2 {
            let l = get2(largest, k);
            if l.is_finite() && l > 0.0 {
                set2(&mut scale, k, l);
            }
        }
    }
    if matches!(face.surface, Surface::Cylinder(_) | Surface::Cone(_)) {
        // Straight along v: how far apart two points are in v does not
        // matter for how well a triangle fits, only how far apart they
        // are around the axis. Squeeze v until the whole face is half a
        // grid column high, so that the triangulation joins points that
        // are neighbours around the axis, along the rulings, also where
        // the two ends of a wall are not parallel.
        let [sa, sb] = scale;
        let [da, _] = step;
        let [_, height] = span;
        let (column, tall) = (0.5 * da * sa, height * sb);
        if column > 0.0 && tall > column {
            scale = [sa, sb * column / tall];
        }
    }
    // With the angle criterion, a torus or a sphere is triangulated in
    // the plane of its grid: each parameter scaled so that a cell of the
    // grid is square. The grid is sized per direction (the chordal
    // tolerance and the angle, each in its own direction), and on a thin
    // fillet the two differ by a hundred: around the axis the surface's
    // normal turns slowly over a long way, across the tube fast over a
    // short one. Scaled to lengths in space such a cell is a sliver, and
    // every point the refinement put in one (its diagonal turns the
    // normal by more than the angle) was joined by Delaunay's rule to
    // points rows away, whose sides turned too far again: the stripes
    // and flange roots of the turbofan had a hundred times the triangles
    // they needed (2026-10-15). In the grid's plane a point added stays
    // among its neighbours. Without the angle the plane is as it was, so
    // that export meshes do not change.
    let mut stretch = [1.0_f64; 2];
    if params.max_angle.is_some() && matches!(face.surface, Surface::Torus(_) | Surface::Sphere(_))
    {
        let ([sa, sb], [da, db]) = (scale, step);
        let square = sa * da / db;
        if square.is_finite() && square > 0.0 && sb > 0.0 {
            stretch = [1.0, square / sb];
            scale = [sa, square];
        }
    }
    let centre = [
        0.5 * (get2(low, 0) + get2(high, 0)),
        0.5 * (get2(low, 1) + get2(high, 1)),
    ];
    let warp = if param.is_freeform() {
        angle_warp(&param, low, high, centre, cells, params)
    } else {
        None
    };
    Ok(Boundary {
        param,
        region,
        low,
        high,
        cells,
        step,
        warp,
        scale,
        stretch,
        centre,
        limit,
    })
}

/// Triangulates face `index`, given its boundary.
///
/// `splits` are sides the mesh must not have: segments between two points
/// of the boundary that the mesh of another face has as well
/// (`body.rs`, `shared_rungs`). On a curved face each gets a point of the
/// surface in its middle; a plane face is left as it is.
///
/// `ask` gets the chords of edges that the face cannot be meshed along as
/// they are, when that is why it fails (the error is then a
/// [`TessError::Face`]): a triangle that stands on a chord and cannot be
/// made to face the way the surface does however close to the chord its
/// third corner comes. That is a chord that lies beside the face more
/// than beside the surface, as one along a rim curved far more tightly
/// across the edge than the tolerance (the nose of a slender aerofoil
/// along a loft): within the tolerance of its curve, and still over the
/// next face. Cut in two, it lies four times nearer to the surface.
/// `retry`: the caller cuts such chords and calls again (not in its last
/// round), so a face with a warp ([`angle_warp`]) asks for them rather
/// than fall back to the plane of its scales.
pub(crate) fn mesh_face(
    boundary: &Boundary,
    face: &Face,
    index: usize,
    params: &TessParams,
    splits: &[(Vec3, Vec3)],
    ask: &mut Vec<Chord>,
    retry: bool,
) -> Result<FaceMesh, TessError> {
    if boundary.warp.is_none() {
        return mesh_face_in(boundary, face, index, params, splits, ask, false);
    }
    // The plane of the warp first. Where it fails for chords of the edges
    // that its triangles cannot follow (along a nose that turns within a
    // few cells), those chords are asked for when the caller can cut them
    // and try again (`retry`), as in the plane of the scales. Otherwise,
    // where it fails (a boundary that the warp makes cross itself, or
    // anything else), the plane of the scales, as without the warp. A
    // budget exceeded is the face's, in either plane.
    let before = ask.len();
    match mesh_face_in(boundary, face, index, params, splits, ask, true) {
        Err(error)
            if !matches!(error, TessError::OverBudget { .. }) && !(retry && ask.len() > before) =>
        {
            ask.truncate(before);
            mesh_face_in(boundary, face, index, params, splits, ask, false)
        }
        done => done,
    }
}

/// [`mesh_face`] in the plane of the boundary's warp (`warped`), or of
/// its scales.
fn mesh_face_in(
    boundary: &Boundary,
    face: &Face,
    index: usize,
    params: &TessParams,
    splits: &[(Vec3, Vec3)],
    ask: &mut Vec<Chord>,
    warped: bool,
) -> Result<FaceMesh, TessError> {
    let (param, region) = (boundary.param, &boundary.region);
    let warp = boundary.warp.as_ref().filter(|_| warped);
    if let Some(doubt) = &region.doubt {
        return Err(doubt.clone());
    }
    let (low, high, cells, step) = (boundary.low, boundary.high, boundary.cells, boundary.step);
    let (scale, centre, limit) = (boundary.scale, boundary.centre, boundary.limit);
    let tolerance = params.chordal_tolerance;

    // The boundary must be closed in parameter space as it is in space. A
    // chord is taken for the straight segment between its ends in
    // parameter space, so the surface along that segment must be near the
    // chord. Where it is further from it than the chord is long (and than
    // the tolerance allows), the two ends are neighbours in space and far
    // apart in parameters: the boundary jumps across the surface, as it
    // does over the seam of a surface that closes on itself without a
    // period known here. What is triangulated then is not the face.
    // (Chords the parameters merely distort were cut finer before this,
    // `Boundary::chords_to_refine`; they end up shorter than they stray.)
    if !param.is_plane() {
        for (n, &(i, j)) in region.segs.iter().enumerate() {
            let Some((edge, _)) = region.chords.get(n).copied().flatten() else {
                continue;
            };
            let (Some(p), Some(q)) = (region.verts.get(i), region.verts.get(j)) else {
                continue;
            };
            let ([pa, pb], [qa, qb]) = (p.uv, q.uv);
            let image = param.eval([0.5 * (pa + qa), 0.5 * (pb + qb)]);
            let off = distance_to_chord(image, p.pos, q.pos);
            if exceeds(off, 2.0 * tolerance) && exceeds(off, p.pos.distance(q.pos)) {
                return fail(
                    index,
                    format!(
                        "its boundary is not closed in parameter space: between two \
                         neighbouring points of edge {edge} it jumps across the surface (the \
                         surface closes on itself in a way that is not recognised, or the edge \
                         does not lie on it)"
                    ),
                );
            }
        }
    }

    let explain = |error: CdtError| -> TessError {
        let reason = match error {
            CdtError::NonFinite => "its boundary has points that are not finite",
            CdtError::Degenerate => {
                "its boundary cannot be triangulated: points of it coincide or its loops \
                 enclose no area in parameter space"
            }
            CdtError::Crossing => {
                "its loops do not bound a region: the boundary crosses itself in parameter space"
            }
            CdtError::Open => {
                "its loops do not bound a region: the boundary is not closed in parameter space"
            }
            CdtError::TooMany => {
                return TessError::TooFine {
                    patch: index,
                    needed_segments: f64::from(params.max_segments) * 2.0 + 2.0,
                    max_segments: params.max_segments,
                };
            }
            CdtError::Internal => {
                return TessError::Internal(format!(
                    "face {index}: the triangulation lost track of its own triangles"
                ));
            }
            CdtError::TooSlow => SLOW_TRIANGULATION,
        };
        TessError::Face {
            face: index,
            reason: reason.into(),
        }
    };

    let corners = match warp {
        Some(([wa, wb], _)) => (
            (wa.map(low[0]), wb.map(low[1])),
            (wa.map(high[0]), wb.map(high[1])),
        ),
        None => (
            (
                (low[0] - centre[0]) * scale[0],
                (low[1] - centre[1]) * scale[1],
            ),
            (
                (high[0] - centre[0]) * scale[0],
                (high[1] - centre[1]) * scale[1],
            ),
        ),
    };
    let mut work = Work {
        param,
        cdt: Cdt::new(corners.0, corners.1, limit).map_err(explain)?,
        uv: Vec::new(),
        pos: Vec::new(),
        free: Vec::new(),
        centre,
        scale,
        warp: warp.map(|(planes, _)| planes.clone()),
        tolerance,
        angle: params.max_angle.filter(|_| !param.is_plane()),
        angle_floor: crate::ANGLE_FLOOR * tolerance,
        normals: Vec::new(),
    };
    // With a triangle budget, the face stops as soon as it is certain to
    // have more triangles than that on its own. The count it gives is
    // then a floor of what it would have had: the region in parameter
    // space is a polygon with holes, and a triangulation of it with `i`
    // points inside has `2 i + b + 2 h - 2` triangles (`b` points on the
    // boundary, `h` holes), of which at most `b` lose their area at a
    // pole (each has a side of the boundary between two points that are
    // one point in space, and each such side has one triangle). So the
    // mesh has at least `2 i - 2` triangles, and every point added
    // (seeding or refinement) is inside. A function of the face and the
    // parameters, as the order of the points is.
    let over_budget = |interior: usize| -> Option<TessError> {
        let max_triangles = params.max_triangles?;
        let floor = interior.saturating_mul(2).saturating_sub(2);
        (floor > max_triangles).then_some(TessError::OverBudget {
            triangles: floor,
            faces: 1,
            max_triangles,
        })
    };

    // Boundary points, in an order that jumps around the boundary (a
    // fixed stride near the golden ratio of their number): inserting
    // neighbours one after the other makes long thin fans that the next
    // point has to undo.
    let count = region.verts.len();
    let mut stride = ((count as f64) * 0.618) as usize;
    let coprime = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a == 1
    };
    while stride > 1 && !coprime(stride, count) {
        stride -= 1;
    }
    let stride = stride.max(1);
    let mut vertex_of: Vec<u32> = vec![0; count];
    for n in 0..count {
        let i = (n * stride) % count;
        let Some(&v) = region.verts.get(i) else {
            continue;
        };
        let id = work
            .cdt
            .insert_boundary(work.plane(v.uv))
            .map_err(explain)?;
        match work.vertex(id) {
            None => work.record(id, v.uv, v.pos, v.free).map_err(explain)?,
            Some((_, pos, _)) => {
                if pos != v.pos {
                    return fail(
                        index,
                        "its boundary passes twice through one place on the surface with \
                         different points",
                    );
                }
            }
        }
        if let Some(slot) = vertex_of.get_mut(i) {
            *slot = id;
        }
    }
    for &(i, j) in &region.segs {
        let (Some(&a), Some(&b)) = (vertex_of.get(i), vertex_of.get(j)) else {
            continue;
        };
        work.cdt.constrain(a, b).map_err(explain)?;
    }
    // When the refinement cannot go on at the triangles `stuck`: the
    // chords of edges they stand on, or lie flat against, go to `ask`,
    // and the error says so; without such chords, `None`. (Cut after cut
    // towards a chord that the surface turns away from, the third corner
    // ends within rounding of it, and the triangle may by then be one
    // row away from the chord: flat against it, not on it.)
    let along_chords = |work: &Work, stuck: &[usize], ask: &mut Vec<Chord>| -> Option<TessError> {
        let before = ask.len();
        for &t in stuck {
            let Some(view) = work.cdt.triangle(t) else {
                continue;
            };
            let [x, y, z] = view.v;
            for (constrained, (p, q)) in view.constrained.into_iter().zip([(y, z), (z, x), (x, y)])
            {
                if !constrained {
                    continue;
                }
                let found = region
                    .segs
                    .iter()
                    .zip(&region.chords)
                    .find_map(|(&(i, j), chord)| {
                        let (a, b) = (vertex_of.get(i).copied()?, vertex_of.get(j).copied()?);
                        ((a, b) == (p, q) || (a, b) == (q, p)).then_some(*chord)?
                    });
                if let Some(chord) = found {
                    ask.push(chord);
                }
            }
            let [Some(a), Some(b), Some(c)] = view.v.map(|v| work.vertex(v)) else {
                continue;
            };
            let corners = [a.0, b.0, c.0].map(|uv| work.plane(uv));
            let longest = corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .map(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
                .fold(0.0_f64, f64::max);
            let mut flat_against: Option<(f64, Chord)> = None;
            // Failing that, the nearest chord whose bulge the triangle
            // lies within (see below).
            let mut in_bulge: Option<(f64, Chord)> = None;
            for (&(i, j), chord) in region.segs.iter().zip(&region.chords) {
                let (Some(chord), Some(p), Some(q)) =
                    (chord, region.verts.get(i), region.verts.get(j))
                else {
                    continue;
                };
                let (from, to) = (work.plane(p.uv), work.plane(q.uv));
                let off = corners
                    .iter()
                    .map(|&corner| plane_distance_to_segment(corner, from, to))
                    .fold(0.0_f64, f64::max);
                if off <= FLAT_AGAINST * longest && flat_against.is_none_or(|(o, _)| off < o) {
                    flat_against = Some((off, *chord));
                }
                // A chord whose middle the surface passes beside, along
                // the surface (a chord of a circle in a tangent plane at
                // the rim of a fillet, which bulges out of the face by
                // its sag): a point of the face nearer to it than that
                // lies beyond it in space, and every triangle between the
                // two faces backwards. Cut after cut, the refinement
                // moves towards the chord without end. (The plane is
                // scaled to lengths in space.)
                if in_bulge.is_none_or(|(o, _)| off < o)
                    && chord_bulge(&work.param, p, q).is_some_and(|aside| off <= NEAR_BULGE * aside)
                {
                    in_bulge = Some((off, *chord));
                }
            }
            if let Some((_, chord)) = flat_against.or(in_bulge) {
                ask.push(chord);
            }
        }
        let (edge, _) = *ask.get(before)?;
        Some(TessError::Face {
            face: index,
            reason: format!(
                "the surface turns away from the chords of edge {edge} faster than triangles \
                 on them can follow at this tolerance; use a finer tolerance"
            ),
        })
    };
    // When the refinement cannot go on at the triangles `stuck` and no
    // chord is to blame: a tolerance finer than the numbers resolve
    // (the triangle is still as large as the tolerance) is `TooFine`
    // without a count (`needed_segments` infinite), as a coarser one
    // cures it; otherwise the triangles are far smaller
    // than the tolerance and still fail, which no tolerance cures, and
    // the error says so instead of asking for segments it never counted.
    let unresolved = |work: &Work, stuck: &[usize], settled: bool| -> TessError {
        let longest = stuck.first().and_then(|&t| {
            let view = work.cdt.triangle(t)?;
            let [Some(a), Some(b), Some(c)] = view.v.map(|v| work.vertex(v)) else {
                return None;
            };
            Some(
                a.1.distance(b.1)
                    .max(b.1.distance(c.1))
                    .max(c.1.distance(a.1)),
            )
        });
        match longest {
            Some(longest) if longest < work.tolerance => TessError::Face {
                face: index,
                reason: format!(
                    "{}: a triangle {longest:.1e} across, far within the tolerance of \
                     {:.1e}, is still off the surface or faces against it (the surface folds \
                     or turns there faster than its parameters resolve)",
                    if settled {
                        "its refinement did not settle"
                    } else {
                        "its refinement cannot go on"
                    },
                    work.tolerance
                ),
            },
            _ => TessError::TooFine {
                patch: index,
                needed_segments: f64::INFINITY,
                max_segments: params.max_segments,
            },
        }
    };
    if work.cdt.classify().map_err(explain)? == 0 {
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }
    // What was added so far is the boundary.
    let boundary_vertices = work.uv.len();

    if !work.param.is_plane() {
        // Seed with the grid, away from the boundary: a point just inside
        // a boundary chord makes a triangle that stands on its side.
        // On the grid of the warp, each cell a unit square of its plane.
        let lines = warp.map(|(_, lines)| lines);
        let [columns, rows] = match lines {
            Some(lines) => lines
                .each_ref()
                .map(|line| u32::try_from(line.len().saturating_sub(1)).unwrap_or(0)),
            None => cells,
        };
        let plane_step = match lines {
            Some(_) => 1.0,
            None => (step[0] * scale[0]).min(step[1] * scale[1]),
        };
        for (i, j) in grid_order(columns, rows) {
            let uv = match lines {
                Some([along_a, along_b]) => {
                    let (Some(&a), Some(&b)) = (along_a.get(i as usize), along_b.get(j as usize))
                    else {
                        continue;
                    };
                    [a, b]
                }
                None => [
                    low[0] + step[0] * f64::from(i),
                    low[1] + step[1] * f64::from(j),
                ],
            };
            let added = work
                .cdt
                .insert_interior(work.plane(uv), 0.3 * plane_step)
                .map_err(explain)?;
            if let Interior::Added(v) = added {
                let pos = work.param.eval(uv);
                work.record(v, uv, pos, None).map_err(explain)?;
                if let Some(over) = over_budget(work.uv.len() - boundary_vertices) {
                    return Err(over);
                }
            }
        }

        // The sides this mesh may not have, each cut in its middle (the
        // triangles change with every cut: look again after each).
        for _ in 0..splits.len() {
            let mut cut = None;
            'search: for t in 0..work.cdt.triangle_count() {
                let Some(view) = work.cdt.triangle(t).filter(|view| view.inside) else {
                    continue;
                };
                let [Some(a), Some(b), Some(c)] = view.v.map(|v| work.vertex(v)) else {
                    continue;
                };
                let [c0, c1, c2] = view.constrained;
                let sides = [(c0, b.1, c.1), (c1, c.1, a.1), (c2, a.1, b.1)];
                for (i, (constrained, from, to)) in sides.into_iter().enumerate() {
                    let named =
                        |&(p, q): &(Vec3, Vec3)| (p == from && q == to) || (p == to && q == from);
                    if !constrained && splits.iter().any(named) {
                        cut = Some((t, i));
                        break 'search;
                    }
                }
            }
            let Some((t, i)) = cut else { break };
            match work.split_edge(t, i) {
                Ok(()) => {}
                // No room for a point there: `body.rs` finds the side
                // still shared and says so.
                Err(CdtError::Degenerate) => break,
                Err(e) => return Err(explain(e)),
            }
        }

        // Measure every triangle, improve the ones that fail, and again
        // for the triangles that changed, until none fails.
        let mut measured: Vec<u32> = Vec::new();
        let mut done = false;
        for _ in 0..200 {
            let total = work.cdt.triangle_count();
            measured.resize(total, u32::MAX);
            let mut failing: Vec<(usize, u32, Action)> = Vec::new();
            for t in 0..total {
                let Some(view) = work.cdt.triangle(t) else {
                    continue;
                };
                if !view.inside || measured.get(t) == Some(&view.generation) {
                    continue;
                }
                match work.assess(t) {
                    Action::Keep => {
                        if let Some(slot) = measured.get_mut(t) {
                            *slot = view.generation;
                        }
                    }
                    action => failing.push((t, view.generation, action)),
                }
            }
            if failing.is_empty() {
                done = true;
                break;
            }
            if let Some(over) = over_budget(work.uv.len() - boundary_vertices) {
                return Err(over);
            }
            for (t, generation, action) in failing {
                // Changed by an earlier split in this round: next round.
                if work.cdt.triangle(t).map(|view| view.generation) != Some(generation) {
                    continue;
                }
                let split = match action {
                    Action::SplitEdge(i) => match work.split_edge(t, i) {
                        Err(CdtError::Degenerate) => work.split_triangle(t),
                        other => other,
                    },
                    Action::Turn(i) => match work.split_edge(t, i) {
                        // No room for a point: the angle is a wish, the
                        // triangle stays as it is.
                        Err(CdtError::Degenerate) => {
                            if let Some(slot) = measured.get_mut(t) {
                                *slot = generation;
                            }
                            Ok(())
                        }
                        other => other,
                    },
                    _ => work.split_triangle(t),
                };
                match split {
                    Ok(()) => {}
                    // No room left for a point between its corners: the
                    // tolerance is beyond what the numbers can resolve,
                    // or the triangle was squeezed against a chord that
                    // the surface turns away from (`ask`).
                    Err(CdtError::Degenerate) => {
                        // This one first, then the others still failing.
                        let stuck: Vec<usize> = std::iter::once(t)
                            .chain(
                                (0..work.cdt.triangle_count())
                                    .filter(|&s| s != t && !matches!(work.assess(s), Action::Keep)),
                            )
                            .take(STUCK_LOOKED_AT)
                            .collect();
                        return Err(along_chords(&work, &stuck, ask)
                            .unwrap_or_else(|| unresolved(&work, &stuck, false)));
                    }
                    Err(e) => return Err(explain(e)),
                }
            }
        }
        if !done {
            // (A few are enough to name the chords; each is held against
            // every segment of the boundary.)
            let stuck: Vec<usize> = (0..work.cdt.triangle_count())
                .filter(|&t| !matches!(work.assess(t), Action::Keep))
                .take(STUCK_LOOKED_AT)
                .collect();
            return Err(
                along_chords(&work, &stuck, ask).unwrap_or_else(|| unresolved(&work, &stuck, true))
            );
        }
    }

    // The triangles, facing outward, over the vertices they use.
    let flip = work.param.swapped != face.reversed;
    let outward = if face.reversed { -1.0 } else { 1.0 };
    let mut mesh = FaceMesh::default();
    let mut index_of: Vec<u32> = vec![u32::MAX; work.uv.len()];
    // Empty for every surface but a spline surface with a pole.
    let poles = craft3d_geom::pole::poles(work.param.surface);
    for t in 0..work.cdt.triangle_count() {
        let Some(view) = work.cdt.triangle(t).filter(|view| view.inside) else {
            continue;
        };
        let [Some(a), Some(b), Some(c)] = view.v.map(|v| work.vertex(v)) else {
            return Err(explain(CdtError::Internal));
        };
        // The sides on the boundary of the region, also those of a
        // triangle that is dropped below: in space its two other sides
        // are one segment, and what lies beyond the one that is no
        // constraint ends on it.
        let [c0, c1, c2] = view.constrained;
        let [va, vb, vc] = view.v;
        let on_boundary = |v: u32| {
            v.checked_sub(FIRST_VERTEX)
                .is_some_and(|i| (i as usize) < boundary_vertices)
        };
        let sides = [
            (c0, (b, c), (vb, vc)),
            (c1, (c, a), (vc, va)),
            (c2, (a, b), (va, vb)),
        ];
        for (constrained, (from, to), (v, w)) in sides {
            if from.1 == to.1 {
                continue;
            }
            if constrained {
                mesh.border
                    .push(if flip { (to.1, from.1) } else { (from.1, to.1) });
            } else if v < w && on_boundary(v) && on_boundary(w) {
                // (Each such side is seen from both its triangles.)
                mesh.rungs.push((from.1, to.1));
            }
        }
        if a.1 == b.1 || b.1 == c.1 || c.1 == a.1 {
            // One point in space on two corners (a pole): no area, and
            // the neighbours on either side already meet there.
            continue;
        }
        let area = (b.1 - a.1).cross(c.1 - a.1).length_squared();
        if !(area > 0.0 && area.is_finite()) {
            return fail(
                index,
                "its surface folds three distinct points onto one line (a triangle without area)",
            );
        }
        let mut corners = [0_u32; 3];
        for (slot, v) in corners.iter_mut().zip(view.v) {
            let i = v
                .checked_sub(FIRST_VERTEX)
                .map_or(usize::MAX, |i| i as usize);
            let (Some(known), Some(vertex)) = (index_of.get_mut(i), work.vertex(v)) else {
                return Err(explain(CdtError::Internal));
            };
            if *known == u32::MAX {
                *known =
                    u32::try_from(mesh.positions.len()).map_err(|_| TessError::TooManyVertices)?;
                mesh.positions.push(vertex.1);
                let normal = work.param.vertex_normal(vertex.0, &poles);
                mesh.normals
                    .push(normal.map_or(Vec3::ZERO, |n| n * outward));
            }
            *slot = *known;
        }
        let [a, b, c] = corners;
        mesh.triangles
            .push(if flip { [a, c, b] } else { [a, b, c] });
    }
    if mesh.triangles.is_empty() {
        return fail(
            index,
            "its loops do not bound a region: they enclose no area on the surface",
        );
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::NurbsSurface;
    use std::f64::consts::FRAC_PI_4;

    /// Nine rational quadratic control points `(x, y, w)` of the unit
    /// circle, and their knots.
    fn ring() -> (Vec<(f64, f64, f64)>, Vec<f64>) {
        let weight = FRAC_PI_4.cos();
        let points = (0..9)
            .map(|k| {
                let (sin, cos) = (FRAC_PI_4 * f64::from(k)).sin_cos();
                if k % 2 == 0 {
                    (cos, sin, 1.0)
                } else {
                    (cos / weight, sin / weight, weight)
                }
            })
            .collect();
        let knots = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        (points, knots)
    }

    /// The profile `(ρ, z)` (a clamped spline of `degree` on uniform
    /// knots) turned about the z axis: u around the axis, v along it.
    fn revolved(profile: &[(f64, f64)], degree: usize) -> NurbsSurface {
        let (ring, ring_knots) = ring();
        let spans = profile.len() - degree;
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend(vec![1.0; degree + 1]);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for &(x, y, w) in &ring {
            for &(rho, z) in profile {
                ctrl.push(Vec3::new(rho * x, rho * y, z));
                weights.push(w);
            }
        }
        NurbsSurface::new(
            2,
            degree,
            ring_knots,
            knots,
            9,
            profile.len(),
            ctrl,
            weights,
        )
        .unwrap()
    }

    const VASE: [(f64, f64); 7] = [
        (3.0, 0.0),
        (5.5, 1.0),
        (6.0, 4.0),
        (1.0, 5.5),
        (1.5, 8.0),
        (3.5, 9.0),
        (2.0, 10.0),
    ];

    #[test]
    fn a_net_closes_where_its_first_row_is_its_last() {
        let vase = revolved(&VASE, 3);
        let closure = Closure::of(&Surface::Nurbs(vase.clone()));
        assert_eq!(closure.periods, [Some(1.0), None]);
        assert_eq!(closure.wrap, [Some((0.0, 1.0)), None]);
        // The analytic kinds say so themselves and are not wrapped.
        let cylinder = Surface::Cylinder(
            craft3d_geom::Cylinder::new(craft3d_math::Frame::WORLD, 2.0).unwrap(),
        );
        let closure = Closure::of(&cylinder);
        assert_eq!(closure.periods, [Some(std::f64::consts::TAU), None]);
        assert_eq!(closure.wrap, [None, None]);

        let (count_u, count_v) = vase.counts();
        let ctrl = vase.control_points().to_vec();
        let weights = vase.weights().to_vec();
        assert_eq!(
            net_closes(count_u, count_v, &ctrl, Some(&weights)),
            [true, false]
        );
        // A last row that is off by more than rounding does not close...
        let mut open = ctrl.clone();
        open[(count_u - 1) * count_v + 2] += Vec3::new(1e-6, 0.0, 0.0);
        assert_eq!(
            net_closes(count_u, count_v, &open, Some(&weights)),
            [false, false]
        );
        // ... one off by rounding does ...
        let mut near = ctrl.clone();
        near[(count_u - 1) * count_v + 2] += Vec3::new(1e-12, 0.0, 0.0);
        assert_eq!(
            net_closes(count_u, count_v, &near, Some(&weights)),
            [true, false]
        );
        // ... and weights in another proportion are another curve.
        let mut heavier = weights.clone();
        heavier[(count_u - 1) * count_v + 2] *= 1.5;
        assert_eq!(
            net_closes(count_u, count_v, &ctrl, Some(&heavier)),
            [false, false]
        );
        let scaled: Vec<f64> = weights
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i >= (count_u - 1) * count_v {
                    w * 3.0
                } else {
                    *w
                }
            })
            .collect();
        assert_eq!(
            net_closes(count_u, count_v, &ctrl, Some(&scaled)),
            [true, false]
        );
        // A net whose every row is one row is a curve, not a tube; and a
        // net that is not what its counts say closes nothing.
        let flat: Vec<Vec3> = (0..count_u * count_v).map(|n| ctrl[n % count_v]).collect();
        assert_eq!(net_closes(count_u, count_v, &flat, None), [false, false]);
        assert_eq!(
            net_closes(count_u, count_v + 1, &ctrl, None),
            [false, false]
        );
        assert_eq!(net_closes(0, 0, &[], None), [false, false]);
        let nan = vec![Vec3::new(f64::NAN, 0.0, 0.0); count_u * count_v];
        assert_eq!(net_closes(count_u, count_v, &nan, None), [false, false]);
        // Knots that are not clamped: the net does not show the closure.
        assert!(clamped(vase.knots_u(), 2));
        assert!(!clamped(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], 2));
        assert!(!clamped(&[], 2));
    }

    #[test]
    fn parameters_beyond_a_closed_spline_surface_are_brought_back() {
        assert_eq!(wrapped(0.25, Some((0.0, 1.0))), 0.25);
        assert_eq!(wrapped(0.0, Some((0.0, 1.0))), 0.0);
        assert_eq!(wrapped(1.0, Some((0.0, 1.0))), 1.0);
        assert_eq!(wrapped(1.25, Some((0.0, 1.0))), 0.25);
        assert_eq!(wrapped(-0.25, Some((0.0, 1.0))), 0.75);
        assert_eq!(wrapped(7.5, Some((2.0, 4.0))), 3.5);
        assert_eq!(wrapped(7.5, None), 7.5);
        assert!(wrapped(f64::NAN, Some((0.0, 1.0))).is_nan());
        assert_eq!(wrapped(f64::INFINITY, Some((0.0, 1.0))), f64::INFINITY);
    }

    /// The cells keep the deviation `(a hu² + b hv²) / 8` within the
    /// tolerance, also where one parameter is held at the limit.
    #[test]
    fn the_cells_of_a_spline_face_keep_the_bound_within_the_tolerance() {
        let within = |(a, b): (f64, f64), span: (f64, f64), tolerance: f64, limit: f64| {
            let (nu, nv) = spline_cells((a, b), span, tolerance, limit)?;
            assert!(nu <= limit && nv <= limit && nu == nu.floor() && nv == nv.floor());
            let (hu, hv) = (span.0 / nu.max(1.0), span.1 / nv.max(1.0));
            let deviation = (a * hu * hu + b * hv * hv) / 8.0;
            assert!(
                deviation <= tolerance * (1.0 + 1e-12),
                "{deviation} for {tolerance}"
            );
            Ok::<_, f64>((nu, nv))
        };
        // Curved alike both ways: the cells of one step for both.
        assert_eq!(
            within((8.0, 8.0), (1.0, 1.0), 1e-2, 1024.0),
            Ok((15.0, 15.0))
        );
        // Straight along v: no cells that way (one, when it comes to it).
        assert_eq!(
            within((8.0, 0.0), (1.0, 3.0), 1e-2, 1024.0),
            Ok((15.0, 0.0))
        );
        // More curved along u, and a box that is not square.
        assert_eq!(
            within((100.0, 1.0), (0.5, 2.0), 1e-3, 1024.0),
            Ok((80.0, 32.0))
        );
        // Over the limit along u by less than √2, with room along v: u is
        // held at the limit and v made finer for it.
        let even = within((100.0, 1.0), (1.0, 1.0), 2e-5, 1e6).unwrap();
        assert_eq!(even, (1119.0, 112.0));
        let held = within((100.0, 1.0), (1.0, 1.0), 2e-5, 1024.0).unwrap();
        assert_eq!(held.0, 1024.0);
        assert!(held.1 > even.1 && held.1 < 1024.0, "{held:?}");
        let other = within((1.0, 100.0), (1.0, 1.0), 2e-5, 1024.0).unwrap();
        assert_eq!(other, (held.1, held.0));
        // Beyond that, or with no room in the other, it is too fine, and
        // says how fine.
        assert_eq!(within((100.0, 1.0), (1.0, 1.0), 1e-5, 1024.0), Err(1582.0));
        assert_eq!(within((100.0, 99.0), (1.0, 1.0), 2e-5, 1024.0), Err(1119.0));
        assert_eq!(
            within((100.0, 100.0), (1.0, 1.0), 2e-5, 1024.0),
            Err(1119.0)
        );
        // Nothing that is not a number passes for a count.
        assert!(within((f64::INFINITY, 1.0), (1.0, 1.0), 1e-3, 1024.0).is_err());
        assert!(within((f64::NAN, 1.0), (1.0, 1.0), 1e-3, 1024.0).is_err());
        assert!(within((1.0, 1.0), (1.0, 1.0), 0.0, 1024.0).is_err());
    }

    #[test]
    fn a_box_is_cut_into_its_stretches_within_the_range() {
        assert_eq!(stretches(0.2, 0.6, (0.0, 1.0), None), vec![(0.2, 0.6)]);
        assert_eq!(stretches(-0.2, 1.6, (0.0, 1.0), None), vec![(0.0, 1.0)]);
        assert_eq!(
            stretches(0.2, 0.6, (0.0, 1.0), Some((0.0, 1.0))),
            vec![(0.2, 0.6)]
        );
        let across = stretches(0.75, 1.25, (0.0, 1.0), Some((0.0, 1.0)));
        assert_eq!(across, vec![(0.75, 1.0), (0.0, 0.25)]);
        let below = stretches(-0.5, 0.25, (0.0, 1.0), Some((0.0, 1.0)));
        assert_eq!(below, vec![(0.5, 1.0), (0.0, 0.25)]);
        // A whole turn from anywhere, and anything that is not a number.
        assert_eq!(stretches(0.3, 1.3, (0.0, 1.0), Some((0.0, 1.0))).len(), 2);
        assert_eq!(
            stretches(0.0, 5.0, (0.0, 1.0), Some((0.0, 1.0))),
            vec![(0.0, 1.0)]
        );
        assert_eq!(stretches(f64::NAN, 1.0, (0.0, 1.0), None), vec![(0.0, 1.0)]);
    }

    /// The largest second differences of `surface` on a lattice over the
    /// box, as `(Muu, Muv, Mvv)`: what the bounds must not be below.
    fn sampled(
        surface: &NurbsSurface,
        closure: &Closure,
        low: (f64, f64),
        high: (f64, f64),
    ) -> (f64, f64, f64) {
        const SAMPLES: u32 = 40;
        let (hu, hv) = (
            (high.0 - low.0) / f64::from(2 * SAMPLES),
            (high.1 - low.1) / f64::from(2 * SAMPLES),
        );
        let [wrap_u, wrap_v] = closure.wrap;
        let (mut muu, mut muv, mut mvv) = (0.0_f64, 0.0_f64, 0.0_f64);
        for i in 0..SAMPLES {
            for j in 0..SAMPLES {
                let u = low.0 + hu * f64::from(2 * i + 1);
                let v = low.1 + hv * f64::from(2 * j + 1);
                let s = |du: f64, dv: f64| {
                    surface.eval(wrapped(u + du * hu, wrap_u), wrapped(v + dv * hv, wrap_v))
                };
                let centre = s(0.0, 0.0);
                muu = muu.max((s(1.0, 0.0) + s(-1.0, 0.0) - centre * 2.0).length() / (hu * hu));
                mvv = mvv.max((s(0.0, 1.0) + s(0.0, -1.0) - centre * 2.0).length() / (hv * hv));
                let mixed = s(1.0, 1.0) - s(1.0, -1.0) - s(-1.0, 1.0) + s(-1.0, -1.0);
                muv = muv.max(mixed.length() / (4.0 * hu * hv));
            }
        }
        (muu, muv, mvv)
    }

    /// The bounds the seed grid is sized by are bounds: nowhere below the
    /// second differences of the surface, on whole surfaces, on parts of
    /// one knot span, across knots and across the seam; and not so far
    /// above them that the grid is wasted.
    #[test]
    fn the_bounds_on_a_spline_surface_are_above_its_second_differences() {
        let wavy = {
            // A polynomial B-spline sheet, 6 by 5 control points.
            let knots = |degree: usize, count: usize| {
                let spans = count - degree;
                let mut knots = vec![0.0; degree + 1];
                knots.extend((1..spans).map(|i| i as f64 / spans as f64));
                knots.extend(vec![1.0; degree + 1]);
                knots
            };
            let mut ctrl = Vec::new();
            for i in 0..6 {
                for j in 0..5 {
                    let z = f64::from((i * 7 + j * 3) % 5) - 2.0;
                    ctrl.push(Vec3::new(f64::from(i), f64::from(j), 0.4 * z));
                }
            }
            let weights = vec![1.0; 30];
            NurbsSurface::new(3, 2, knots(3, 6), knots(2, 5), 6, 5, ctrl, weights).unwrap()
        };
        let surfaces = [
            ("vase", revolved(&VASE, 3)),
            ("vase of degree 5", revolved(&VASE, 5)),
            (
                "barrel",
                revolved(&[(1.0, 0.0), (1.3, 0.5), (1.3, 1.5), (1.0, 2.0)], 3),
            ),
            ("wavy sheet", wavy),
        ];
        let boxes = [
            ((0.0, 0.0), (1.0, 1.0)),
            ((0.1, 0.2), (0.2, 0.9)),
            ((0.2, 0.1), (0.8, 0.3)),
            ((0.26, 0.51), (0.27, 0.52)),
            ((0.24, 0.24), (0.26, 0.26)),
            ((0.5, 0.0), (0.5 + 1e-7, 1.0)),
        ];
        let mut loosest = 0.0_f64;
        for (name, surface) in &surfaces {
            let closure = Closure::of(&Surface::Nurbs(surface.clone()));
            let mut all = boxes.to_vec();
            if closure.wrap[0].is_some() {
                // Across the seam, as a face that goes around is laid out.
                all.push(((0.7, 0.0), (1.7, 1.0)));
                all.push(((0.9, 0.3), (1.2, 0.6)));
                all.push(((-0.3, 0.3), (0.1, 0.6)));
            }
            for (low, high) in all {
                let (muu, muv, mvv) = spline_second_bounds(surface, &closure, low, high);
                let (suu, suv, svv) = sampled(surface, &closure, low, high);
                let what = format!("{name} over {low:?}..{high:?}");
                for (bound, seen, which) in [(muu, suu, "uu"), (muv, suv, "uv"), (mvv, svv, "vv")] {
                    assert!(bound.is_finite(), "{what}: no bound on {which}");
                    // (Second differences are means of the derivative,
                    // so they do not exceed its bound but for rounding;
                    // a box narrower than the lattice can resolve, the
                    // last one, has none worth the name.)
                    let resolved = high.0 - low.0 > 1e-6;
                    assert!(
                        !resolved || bound >= seen * (1.0 - 1e-6) - 1e-6,
                        "{what}: the bound {bound} on {which} is below {seen}"
                    );
                }
                // How loose: the cells of the grid go by the root of this.
                if high.0 - low.0 >= 0.3 && high.1 - low.1 >= 0.3 {
                    let ratio = (muu + 2.0 * muv + mvv) / (suu + 2.0 * suv + svv);
                    println!("{what}: bound / seen = {ratio:.2}");
                    loosest = loosest.max(ratio);
                }
            }
        }
        println!("loosest: {loosest:.2}");
    }
}
