//! Extrusion up to a face: the solid swept by profiles along the normal of
//! their plane until it meets the surface a face lies on.
//!
//! **Method.** The profiles are extruded past the surface (the *long
//! prism*), a *tool* solid is built that has the surface as one of its
//! faces, and the two are combined by [`crate::boolean`]: the prism minus
//! the tool when the sketch plane is outside the tool, the prism inside the
//! tool when it is inside (a sketch plane within a ball, extruded up to the
//! ball's inside). Of the result the part that hangs on to the start caps
//! is kept: the prism up to where it first meets the surface. What lies
//! beyond (a prism that goes through a cylinder comes out on the other
//! side) is dropped.
//!
//! **The tool** per kind of surface:
//!
//! | Surface | Tool |
//! |---|---|
//! | plane | a block behind the plane, large enough to hold the rest of the prism |
//! | cylinder | the solid cylinder, long enough to hold the prism |
//! | sphere, torus | the solid ball, the solid ring |
//! | polynomial Bézier patch or B-spline surface | the solid behind the whole patch: the patch, a wall under each of its four borders along the direction, and a flat far side |
//! | cone; a rational freeform surface | not built yet: [`KernelError::Unsupported`] |
//!
//! So the end lies on the *surface* of the face: a plane, a cylinder, a
//! sphere and a torus continue beyond the face's own boundary, a freeform
//! surface ends where its patch ends. The profile has to reach the face
//! itself somewhere (checked on sampled points, see [`reaches`]); where it
//! hangs over the face's boundary it ends on the surface continued.
//!
//! **Refused**, each with its reason ([`KernelError::NoEnd`]): a surface
//! parallel to the direction (or, for a freeform patch, one that turns away
//! from it somewhere); a surface behind the sketch plane; a surface that
//! passes through the sketch plane inside a profile; a profile that passes
//! beside the surface or reaches beyond the patch, so that part of it never
//! ends; a profile that misses the face. What the boolean refuses is
//! returned as it is.
//!
//! **Origins.** The faces are reported in the roles of a plain extrusion:
//! the cap in the sketch plane is the start cap, the walls are sides, and
//! the faces on the target surface are the end cap of their profile (more
//! than one face when the boolean left the cap in pieces). Edges and
//! vertices are reported likewise when each can be told (a piece of a
//! start edge, of a lateral edge, the end edge of a wall, the end of a
//! lateral edge); otherwise both lists are empty and edges are referred to
//! by the faces they lie between.

use crate::boolean::{
    BooleanOp, BooleanResult, EdgeSource, Entity, Operand, VertexSource, boolean, contains_points,
};
use crate::extrude::{OpResult, Origin, Origins, Role, extrude};
use crate::primitives;
use crate::profile::{Profile, Shape};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check};
use std::collections::BTreeSet;

const OPERATION: &str = "extrude up to a face";

/// Below this sine of the angle between a surface and the direction, the
/// surface counts as parallel to it: the end would lie a million times
/// further away than the profile is wide.
const PARALLEL: f64 = 1e-6;

/// Samples per edge when looking whether the profile reaches the face.
const EDGE_SAMPLES: u32 = 9;

/// Steps each way of the grid on which a freeform patch is tested for
/// facing the direction everywhere.
const FACING_GRID: u32 = 16;

/// The face an extrusion ends on: faces of `body` that lie on one surface
/// (one face, or the pieces a boolean cut one face into).
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    pub body: &'a Body,
    pub faces: &'a [FaceId],
}

fn no_end(reason: &'static str) -> KernelError {
    KernelError::NoEnd { reason }
}

fn unsupported(reason: &'static str) -> KernelError {
    KernelError::Unsupported {
        operation: OPERATION,
        reason,
        faces: Vec::new(),
    }
}

const PARALLEL_FACE: &str = "the face is parallel to the direction of the extrusion";
const BEHIND: &str =
    "the face lies behind the sketch plane; the extrusion goes away from it (`reversed` turns it round)";
const THROUGH_START: &str =
    "the face's surface passes through the sketch plane inside the profile, or the profile starts behind it";
const NEVER_ENDS: &str = "part of the profile passes beside the face's surface, or reaches beyond it, and never ends";
const MISSES: &str = "the profile misses the face: it would end on the face's surface beside the face itself";

/// The box of the profiles in sketch coordinates: `(lowest corner, highest
/// corner)`. Conservative for arcs (the whole circle) and splines (the
/// control points). `None` without profiles or when a coordinate is not
/// finite.
pub fn profile_box(profiles: &[Profile]) -> Option<([f64; 2], [f64; 2])> {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut finite = true;
    let mut add = |p: [f64; 2]| {
        finite &= p.iter().all(|c| c.is_finite());
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    };
    for segment in profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
        .flat_map(|l| &l.segments)
    {
        add(segment.start);
        add(segment.end);
        match &segment.shape {
            Shape::Line => {}
            &Shape::Arc { center, radius } | &Shape::Circle { center, radius } => {
                add([center[0] - radius, center[1] - radius]);
                add([center[0] + radius, center[1] + radius]);
            }
            Shape::Spline(spline) => spline.control.iter().for_each(|p| add(*p)),
        }
    }
    (finite && lo[0] <= hi[0] && lo[1] <= hi[1]).then_some((lo, hi))
}

/// What the boolean is done with.
struct Tool {
    /// A solid whose face 0 lies on the target surface.
    body: Body,
    /// How far the long prism goes: past the surface everywhere.
    far: f64,
    /// True when the sketch plane is inside the tool: the prism is cut to
    /// what lies inside it.
    inside: bool,
}

/// Where the sweep starts and which way it goes.
struct Sweep<'a> {
    plane: &'a Frame,
    /// Unit, along or against the plane's normal.
    dir: Vec3,
    /// The corners of the profiles' box, in the sketch plane.
    corners: [Vec3; 4],
    /// A point of every profile, in the sketch plane.
    starts: Vec<Vec3>,
    /// Room left around everything, so that nothing ends flush.
    margin: f64,
    tol: f64,
}

impl Sweep<'_> {
    /// How far `p` is from the sketch plane in the direction of the sweep.
    fn depth(&self, p: Vec3) -> f64 {
        (p - self.plane.origin()).dot(self.dir)
    }

    /// The corners of the long prism's box when it goes `far`.
    fn reach(&self, far: f64) -> impl Iterator<Item = Vec3> + '_ {
        self.corners
            .iter()
            .flat_map(move |&c| [c, c + self.dir * far])
    }

    /// Whether the sketch plane's profiles start inside the solid whose
    /// signed distance is `distance` (negative inside).
    fn starts_inside(&self, distance: impl Fn(Vec3) -> f64) -> Result<bool, KernelError> {
        let mut inside = None;
        for &p in &self.starts {
            let d = distance(p);
            if d.abs() <= 2.0 * self.tol {
                continue;
            }
            match inside {
                None => inside = Some(d < 0.0),
                Some(side) if side == (d < 0.0) => {}
                Some(_) => return Err(no_end(THROUGH_START)),
            }
        }
        inside.ok_or(no_end(THROUGH_START))
    }

    fn far_enough(&self, far: f64) -> Result<f64, KernelError> {
        if !far.is_finite() {
            return Err(no_end(PARALLEL_FACE));
        }
        if far <= self.tol {
            return Err(no_end(BEHIND));
        }
        Ok(far + self.margin)
    }
}

fn plane_tool(sweep: &Sweep, plane: &Plane) -> Result<Tool, KernelError> {
    let frame = plane.frame();
    let slope = frame.z().dot(sweep.dir);
    if slope.abs() <= PARALLEL {
        return Err(no_end(PARALLEL_FACE));
    }
    // Along the direction, a corner of the box meets the plane at:
    let meets = |c: Vec3| (frame.origin() - c).dot(frame.z()) / slope;
    let last = sweep
        .corners
        .iter()
        .map(|&c| meets(c))
        .fold(f64::NEG_INFINITY, f64::max);
    let far = sweep.far_enough(last)?;
    // The block lies behind the plane, seen along the direction.
    let behind = if slope > 0.0 { frame.z() } else { -frame.z() };
    let lying = Frame::new(frame.origin(), behind, frame.x())
        .ok_or(no_end("the face has a degenerate plane"))?;
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in sweep.reach(far) {
        let (x, y, z) = lying.local(p);
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y), hi[2].max(z)];
    }
    let m = sweep.margin;
    let corner = lying
        .at(lying.point(lo[0] - m, lo[1] - m, 0.0))
        .ok_or(no_end("the face has a degenerate plane"))?;
    let body = primitives::block(
        &corner,
        hi[0] - lo[0] + 2.0 * m,
        hi[1] - lo[1] + 2.0 * m,
        hi[2].max(0.0) + m,
    )?;
    Ok(Tool {
        body,
        far,
        inside: false,
    })
}

fn cylinder_tool(sweep: &Sweep, cylinder: &craft3d_geom::Cylinder) -> Result<Tool, KernelError> {
    let (frame, radius) = (cylinder.frame(), cylinder.radius());
    let axis = frame.z();
    let across = sweep.dir - axis * axis.dot(sweep.dir);
    let q = across.length();
    if q <= PARALLEL {
        return Err(no_end(PARALLEL_FACE));
    }
    // A line along the direction is nearest the axis at this depth, and
    // has left the cylinder `radius / q` further on at the latest.
    let nearest = |c: Vec3| {
        let w = c - frame.origin();
        -(w - axis * axis.dot(w)).dot(across) / (q * q)
    };
    let last = sweep
        .corners
        .iter()
        .map(|&c| nearest(c))
        .fold(f64::NEG_INFINITY, f64::max)
        + radius / q;
    let far = sweep.far_enough(last)?;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in sweep.reach(far) {
        let along = (p - frame.origin()).dot(axis);
        (low, high) = (low.min(along), high.max(along));
    }
    let m = sweep.margin + radius;
    let base = frame
        .at(frame.point(0.0, 0.0, low - m))
        .ok_or(no_end("the face has a degenerate cylinder"))?;
    let body = primitives::cylinder(&base, radius, high - low + 2.0 * m)?;
    let inside = sweep.starts_inside(|p| {
        let w = p - frame.origin();
        (w - axis * axis.dot(w)).length() - radius
    })?;
    Ok(Tool { body, far, inside })
}

fn sphere_tool(sweep: &Sweep, sphere: &craft3d_geom::Sphere) -> Result<Tool, KernelError> {
    let (frame, radius) = (sphere.frame(), sphere.radius());
    let far = sweep.far_enough(sweep.depth(frame.origin()) + radius)?;
    let inside = sweep.starts_inside(|p| p.distance(frame.origin()) - radius)?;
    Ok(Tool {
        body: primitives::sphere(frame, radius)?,
        far,
        inside,
    })
}

fn torus_tool(sweep: &Sweep, torus: &craft3d_geom::Torus) -> Result<Tool, KernelError> {
    let (frame, (major, minor)) = (torus.frame(), torus.radii());
    let far = sweep.far_enough(sweep.depth(frame.origin()) + major + minor)?;
    let inside = sweep.starts_inside(|p| {
        let (x, y, z) = frame.local(p);
        (x.hypot(y) - major).hypot(z) - minor
    })?;
    Ok(Tool {
        body: primitives::torus(frame, major, minor)?,
        far,
        inside,
    })
}

/// A border of a patch: a row or a column of its control points.
struct Border {
    degree: usize,
    knots: Vec<f64>,
    control: Vec<Vec3>,
}

/// True when the knot vector starts and ends with `degree + 1` equal
/// knots: the curve then starts and ends at its end control points.
fn clamped(knots: &[f64], degree: usize) -> bool {
    let ends_equal = |mut it: std::slice::Iter<f64>| {
        let first = it.next();
        first.is_some() && it.take(degree).all(|k| Some(k) == first) && knots.len() > 2 * degree
    };
    let reversed: Vec<f64> = knots.iter().rev().copied().collect();
    ends_equal(knots.iter()) && ends_equal(reversed.iter())
}

/// The solid behind a whole freeform patch, seen along the direction: the
/// patch (face 0), a wall under each of its four borders, and a flat far
/// side square to the direction.
fn freeform_tool(sweep: &Sweep, surface: &Surface) -> Result<Tool, KernelError> {
    let patch = match surface {
        Surface::Bezier(b) => NurbsSurface::from_bezier(b)?,
        Surface::Nurbs(s) => s.clone(),
        _ => return Err(unsupported("this kind of surface as the end")),
    };
    if patch.weights().iter().any(|w| *w != 1.0) {
        return Err(unsupported(
            "a rational freeform face as the end of an extrusion",
        ));
    }
    let (pu, pv) = patch.degrees();
    let (cu, cv) = patch.counts();
    if !(clamped(patch.knots_u(), pu) && clamped(patch.knots_v(), pv)) {
        return Err(unsupported(
            "a freeform face whose surface does not end at its outer control points",
        ));
    }
    // The solid lies behind the patch, so the patch has to face the
    // direction (or face away from it) everywhere: a patch that turns
    // round would make a solid that passes through itself.
    let ((u0, u1), (v0, v1)) = patch.domain();
    let mut facing: Option<bool> = None;
    for i in 0..=FACING_GRID {
        for j in 0..=FACING_GRID {
            let at = |a: f64, b: f64, k: u32| a + (b - a) * f64::from(k) / f64::from(FACING_GRID);
            let Some(normal) = surface.normal(at(u0, u1, i), at(v0, v1, j)) else {
                return Err(unsupported(
                    "a freeform face whose surface has a border or a point without a normal",
                ));
            };
            let s = normal.dot(sweep.dir);
            if s.abs() <= PARALLEL || facing.is_some_and(|f| f != (s > 0.0)) {
                return Err(no_end(
                    "the face's surface is parallel to the direction of the extrusion somewhere, or turns away from it",
                ));
            }
            facing = Some(s > 0.0);
        }
    }
    let Some(along) = facing else {
        return Err(no_end(PARALLEL_FACE));
    };

    let control = patch.control_points();
    let last = control
        .iter()
        .map(|&p| sweep.depth(p))
        .fold(f64::NEG_INFINITY, f64::max);
    let far = sweep.far_enough(last)?;
    let floor = far + sweep.margin;
    let dropped = |p: Vec3| p + sweep.dir * (floor - sweep.depth(p));

    let row = |i: usize| -> Vec<Vec3> {
        (0..cv)
            .filter_map(|j| control.get(i.checked_mul(cv)?.checked_add(j)?).copied())
            .collect()
    };
    let column = |j: usize| -> Vec<Vec3> {
        (0..cu)
            .filter_map(|i| control.get(i.checked_mul(cv)?.checked_add(j)?).copied())
            .collect()
    };
    let along_u = |control: Vec<Vec3>| Border {
        degree: pu,
        knots: patch.knots_u().to_vec(),
        control,
    };
    let along_v = |control: Vec<Vec3>| Border {
        degree: pv,
        knots: patch.knots_v().to_vec(),
        control,
    };
    // Round the patch with its normal (∂u × ∂v) pointing at the viewer:
    // along v = v0, up u = u1, back along v = v1, down u = u0. `true`
    // marks a border the round runs against.
    let mut round = vec![
        (along_u(column(0)), false),
        (along_v(row(cu.saturating_sub(1))), false),
        (along_u(column(cv.saturating_sub(1))), true),
        (along_v(row(0)), true),
    ];
    // Seen from outside the solid, which is from where the sweep comes:
    // the other way round when the normal points along the direction.
    if along {
        round.reverse();
        for (_, against) in &mut round {
            *against = !*against;
        }
    }

    let degenerate = || unsupported("a freeform face with a border that is a single point");
    let mut body = Body::new(BodyKind::Solid);
    // Per border: the vertex the round enters it by, at the patch and at
    // the floor.
    let mut tops = Vec::with_capacity(4);
    let mut bottoms = Vec::with_capacity(4);
    for (border, against) in &round {
        let entry = if *against {
            border.control.last()
        } else {
            border.control.first()
        };
        let &entry = entry.ok_or_else(degenerate)?;
        tops.push(body.add_vertex(entry));
        bottoms.push(body.add_vertex(dropped(entry)));
    }
    let next = |list: &[VertexId], k: usize| list.get((k + 1) % 4).copied();
    let mut top_edges = Vec::with_capacity(4);
    let mut bottom_edges = Vec::with_capacity(4);
    let mut laterals = Vec::with_capacity(4);
    for (k, (border, against)) in round.iter().enumerate() {
        let (Some(&a), Some(b), Some(&c), Some(d)) = (
            tops.get(k),
            next(&tops, k),
            bottoms.get(k),
            next(&bottoms, k),
        ) else {
            return Err(degenerate());
        };
        let (Some(pa), Some(pb), Some(pc)) = (body.vertex(a), body.vertex(b), body.vertex(c))
        else {
            return Err(degenerate());
        };
        let (pa, pb, pc) = (pa.point, pb.point, pc.point);
        if pa.distance(pb) <= 2.0 * sweep.tol {
            return Err(degenerate());
        }
        // Each curve runs its own way: from the round's exit to its entry
        // when the round goes against it.
        let own = |from: VertexId, to: VertexId| if *against { (to, from) } else { (from, to) };
        let top = NurbsCurve::polynomial(
            border.degree,
            border.knots.clone(),
            border.control.clone(),
        )?;
        let range = top.domain();
        let (s, e) = own(a, b);
        top_edges.push(body.add_edge(Curve3::Nurbs(top), range, s, e));
        let bottom = NurbsCurve::polynomial(
            border.degree,
            border.knots.clone(),
            border.control.iter().map(|&p| dropped(p)).collect(),
        )?;
        let (s, e) = own(c, d);
        bottom_edges.push(body.add_edge(Curve3::Nurbs(bottom), range, s, e));
        let line = Curve3::Line(Line::through(pa, pc)?);
        laterals.push(body.add_edge(line, (0.0, pa.distance(pc)), a, c));
    }

    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let mut faces = Vec::with_capacity(6);
    // Face 0: the patch itself.
    let top_loop: Vec<Coedge> = top_edges
        .iter()
        .zip(&round)
        .map(|(&e, (_, against))| coedge(e, *against))
        .collect();
    let top_loop = body.add_loop(top_loop);
    faces.push(body.add_face(surface.clone(), along, vec![top_loop]));
    // The walls: back along the border, down, along the floor, up.
    for (k, (border, against)) in round.iter().enumerate() {
        let (Some(&top), Some(&bottom), Some(&down), Some(&up)) = (
            top_edges.get(k),
            bottom_edges.get(k),
            laterals.get(k),
            laterals.get((k + 1) % 4),
        ) else {
            return Err(degenerate());
        };
        let wall_loop = body.add_loop(vec![
            coedge(top, !*against),
            coedge(down, false),
            coedge(bottom, *against),
            coedge(up, true),
        ]);
        let mut control = Vec::with_capacity(border.control.len() * 2);
        for &p in &border.control {
            control.push(p);
            control.push(dropped(p));
        }
        let weights = vec![1.0; control.len()];
        let wall = NurbsSurface::new(
            border.degree,
            1,
            border.knots.clone(),
            vec![0.0, 0.0, 1.0, 1.0],
            border.control.len(),
            2,
            control,
            weights,
        )?;
        // The wall's normal is the border's own direction crossed with
        // the sweep's: inward when the round runs the border's way.
        faces.push(body.add_face(Surface::Nurbs(wall), !*against, vec![wall_loop]));
    }
    // The floor: the borders dropped, the other way round.
    let floor_loop: Vec<Coedge> = bottom_edges
        .iter()
        .zip(&round)
        .rev()
        .map(|(&e, (_, against))| coedge(e, !*against))
        .collect();
    let floor_loop = body.add_loop(floor_loop);
    let floor_frame = Frame::new(
        sweep.plane.origin() + sweep.dir * floor,
        sweep.dir,
        sweep.plane.x(),
    )
    .ok_or(no_end("the sketch plane is degenerate"))?;
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    ));
    body.add_shell(faces);

    if !check(&body, &Tolerance {
        linear: sweep.tol,
        angular: 1e-9,
    })
    .is_empty()
    {
        return Err(unsupported(
            "a freeform face whose surface, with a wall under each border, does not close into a solid",
        ));
    }
    Ok(Tool {
        body,
        far,
        inside: false,
    })
}

/// What a face of the boolean's result is to the extrusion.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The cap in the sketch plane.
    Near,
    /// The cap at the far end of the long prism.
    Far,
    Side,
    /// On the target surface.
    End,
    /// Another face of the tool.
    Stray,
}

/// The faces and edges of `body` that use each loop's coedges, flattened:
/// the edges of face `face`.
fn edges_of(body: &Body, face: FaceId) -> Vec<EdgeId> {
    body.face(face)
        .into_iter()
        .flat_map(|f| &f.loops)
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .collect()
}

/// Points along the edges of `faces`, a few per edge, clear of the ends.
fn edge_samples(body: &Body, faces: impl Iterator<Item = FaceId>) -> Vec<Vec3> {
    let mut seen = BTreeSet::new();
    let mut points = Vec::new();
    for edge in faces.flat_map(|f| edges_of(body, f)) {
        if !seen.insert(edge) {
            continue;
        }
        let Some(e) = body.edge(edge) else { continue };
        for k in 0..EDGE_SAMPLES {
            let t = (f64::from(k) + 0.5) / f64::from(EDGE_SAMPLES);
            let p = e.curve.eval(e.range.0 + (e.range.1 - e.range.0) * t);
            if p.is_finite() {
                points.push(p);
            }
        }
    }
    points
}

/// Whether the end the boolean made reaches the target face itself, and
/// not only the surface it lies on continued.
///
/// Looked at from both sides, on points sampled along edges: a point of
/// the end's boundary lies on the face's body when a step along the
/// surface's normal leads into the body and a step against it out of it
/// (or the other way round); a point of the face's boundary lies under the
/// profile when it is inside the long prism. One such point is enough. A
/// face without edges (a whole sphere or torus) is its whole surface.
///
/// What this cannot see: an end and a face that overlap without a sampled
/// point of either boundary inside the other (nine points per edge), and
/// a face of the same body on the same surface that is not the target
/// (the two tops of a U-shaped block): the end may land there.
fn reaches(
    target: &Target,
    surface: &Surface,
    end_points: &[Vec3],
    prism: &Body,
    tolerance: &Tolerance,
) -> Result<bool, KernelError> {
    let outline = edge_samples(target.body, target.faces.iter().copied());
    if outline.is_empty() {
        return Ok(true);
    }
    let under = contains_points(prism, &outline, tolerance)?;
    if under.contains(&Some(true)) {
        return Ok(true);
    }
    let step = 8.0 * tolerance.linear;
    let mut probes = Vec::with_capacity(end_points.len() * 2);
    for &p in end_points {
        let Ok((u, v)) = surface.project(p) else {
            continue;
        };
        let Some(normal) = surface.normal(u, v) else {
            continue;
        };
        probes.push(p + normal * step);
        probes.push(p - normal * step);
    }
    let sides = contains_points(target.body, &probes, tolerance)?;
    Ok(sides
        .chunks_exact(2)
        .any(|pair| matches!(pair, [Some(a), Some(b)] if a != b)))
}

/// Extrudes the profiles from `plane` along its normal (against it when
/// `reversed`) until they meet the surface of `target`'s face. See the
/// module documentation for the method, what is refused and the origins.
///
/// The body is a solid with one shell per profile and has passed
/// `craft3d_topo::check` with `tolerance`.
pub fn extrude_up_to(
    profiles: &[Profile],
    plane: &Frame,
    reversed: bool,
    target: &Target,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let Some(surface) = target
        .faces
        .first()
        .and_then(|f| target.body.face(*f))
        .map(|f| &f.surface)
    else {
        return Err(no_end("the face is not a face of its body"));
    };
    let Some((lo, hi)) = profile_box(profiles) else {
        return Err(KernelError::NoProfile);
    };
    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let dir = if reversed { -plane.z() } else { plane.z() };
    let starts: Vec<Vec3> = profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
        .flat_map(|l| &l.segments)
        .map(|s| plane.point(s.start[0], s.start[1], 0.0))
        .collect();
    let sweep = Sweep {
        plane,
        dir,
        corners: [
            plane.point(lo[0], lo[1], 0.0),
            plane.point(hi[0], lo[1], 0.0),
            plane.point(hi[0], hi[1], 0.0),
            plane.point(lo[0], hi[1], 0.0),
        ],
        starts,
        margin: (0.25 * size).max(1000.0 * tol),
        tol,
    };
    if !(sweep.margin.is_finite() && sweep.corners.iter().all(|c| c.is_finite())) {
        return Err(no_end("the profile or the sketch plane is not finite"));
    }
    let tool = match surface {
        Surface::Plane(p) => plane_tool(&sweep, p)?,
        Surface::Cylinder(c) => cylinder_tool(&sweep, c)?,
        Surface::Sphere(s) => sphere_tool(&sweep, s)?,
        Surface::Torus(t) => torus_tool(&sweep, t)?,
        Surface::Cone(_) => {
            return Err(unsupported("a cone face as the end of an extrusion"));
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => freeform_tool(&sweep, surface)?,
    };

    // The kernel's extrusion starts at its lower offset: against the
    // normal that is the far end.
    let (prism, near, far) = if reversed {
        (
            extrude(profiles, plane, -tool.far, 0.0, tolerance)?,
            Role::EndCap,
            Role::StartCap,
        )
    } else {
        (
            extrude(profiles, plane, 0.0, tool.far, tolerance)?,
            Role::StartCap,
            Role::EndCap,
        )
    };
    let op = if tool.inside {
        BooleanOp::Intersect
    } else {
        BooleanOp::Subtract
    };
    let cut = match boolean(&prism.body, &tool.body, op, tolerance) {
        Ok(cut) => cut,
        // Nothing left: the whole prism is on the wrong side.
        Err(KernelError::EmptyResult { .. }) => return Err(no_end(THROUGH_START)),
        Err(other) => return Err(other),
    };

    let part = |index: usize| -> Option<Part> {
        let source = cut.faces.get(index)?;
        Some(match source.operand {
            Operand::B if source.face == FaceId(0) => Part::End,
            Operand::B => Part::Stray,
            Operand::A => match prism.origins.faces.get(source.face.0 as usize)?.role {
                role if role == near => Part::Near,
                role if role == far => Part::Far,
                _ => Part::Side,
            },
        })
    };
    let lost = || no_end("the boolean returned a face without a source");
    let mut parts = Vec::with_capacity(cut.body.faces.len());
    for index in 0..cut.body.faces.len() {
        parts.push(part(index).ok_or_else(lost)?);
    }
    let part_of = |face: &FaceId| parts.get(face.0 as usize).copied();
    // What hangs on to a start cap is the extrusion; the rest lies beyond
    // the surface.
    let kept: Vec<&craft3d_topo::Shell> = cut
        .body
        .shells
        .iter()
        .filter(|shell| shell.faces.iter().any(|f| part_of(f) == Some(Part::Near)))
        .collect();
    let count = |wanted: Part| {
        kept.iter()
            .flat_map(|s| &s.faces)
            .filter(|f| part_of(f) == Some(wanted))
            .count()
    };
    if kept.len() != profiles.len() || count(Part::Near) != profiles.len() {
        return Err(no_end(THROUGH_START));
    }
    if count(Part::Far) > 0
        || count(Part::Stray) > 0
        || kept
            .iter()
            .any(|s| !s.faces.iter().any(|f| part_of(f) == Some(Part::End)))
    {
        return Err(no_end(NEVER_ENDS));
    }
    // A start cap the surface has cut into is not the profile any more.
    let whole = kept
        .iter()
        .flat_map(|s| &s.faces)
        .filter(|f| part_of(f) == Some(Part::Near))
        .flat_map(|f| edges_of(&cut.body, *f))
        .all(|e| {
            matches!(
                cut.edges.get(e.0 as usize),
                Some(EdgeSource::Kept {
                    operand: Operand::A,
                    ..
                })
            )
        });
    if !whole {
        return Err(no_end(THROUGH_START));
    }
    let ends = kept
        .iter()
        .flat_map(|s| &s.faces)
        .filter(|f| part_of(f) == Some(Part::End))
        .copied();
    let end_points = edge_samples(&cut.body, ends);
    if !reaches(target, surface, &end_points, &prism.body, tolerance)? {
        return Err(no_end(MISSES));
    }

    let made = assemble(&cut, &prism.origins, &kept, &parts, near)?;
    let mut defects = check(&made.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        });
    }
    Ok(made)
}

/// The kept shells as a body of their own, with the origin of everything
/// in the roles of a plain extrusion.
fn assemble(
    cut: &BooleanResult,
    prism: &Origins,
    kept: &[&craft3d_topo::Shell],
    parts: &[Part],
    near: Role,
) -> Result<OpResult, KernelError> {
    let lost = || no_end("the boolean returned a body with a dangling reference");
    let old = &cut.body;
    let mut faces = BTreeSet::new();
    let mut loops = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut vertices = BTreeSet::new();
    for face in kept.iter().flat_map(|s| &s.faces) {
        faces.insert(*face);
        for loop_id in &old.face(*face).ok_or_else(lost)?.loops {
            loops.insert(*loop_id);
            for coedge in &old.loop_(*loop_id).ok_or_else(lost)?.coedges {
                edges.insert(coedge.edge);
                let edge = old.edge(coedge.edge).ok_or_else(lost)?;
                vertices.insert(edge.start);
                vertices.insert(edge.end);
            }
        }
    }
    // The new number of an entity is its rank among the kept ones.
    fn rank<T: Ord + Copy>(set: &BTreeSet<T>, id: T) -> Option<u32> {
        u32::try_from(set.range(..id).count()).ok()
    }
    let mut body = Body::new(BodyKind::Solid);
    for vertex in &vertices {
        body.add_vertex(old.vertex(*vertex).ok_or_else(lost)?.point);
    }
    for edge in &edges {
        let e = old.edge(*edge).ok_or_else(lost)?;
        let (Some(start), Some(end)) = (rank(&vertices, e.start), rank(&vertices, e.end)) else {
            return Err(lost());
        };
        body.add_edge(e.curve.clone(), e.range, VertexId(start), VertexId(end));
    }
    for loop_id in &loops {
        let mut coedges = Vec::new();
        for coedge in &old.loop_(*loop_id).ok_or_else(lost)?.coedges {
            coedges.push(Coedge {
                edge: EdgeId(rank(&edges, coedge.edge).ok_or_else(lost)?),
                reversed: coedge.reversed,
            });
        }
        body.add_loop(coedges);
    }
    for face in &faces {
        let f = old.face(*face).ok_or_else(lost)?;
        let mut own = Vec::new();
        for loop_id in &f.loops {
            own.push(craft3d_topo::LoopId(
                rank(&loops, *loop_id).ok_or_else(lost)?,
            ));
        }
        body.add_face(f.surface.clone(), f.reversed, own);
    }
    for shell in kept {
        let mut own = Vec::new();
        for face in &shell.faces {
            own.push(FaceId(rank(&faces, *face).ok_or_else(lost)?));
        }
        body.add_shell(own);
    }

    let part_of = |face: FaceId| parts.get(face.0 as usize).copied();
    let prism_face = |face: FaceId| {
        let source = cut.faces.get(face.0 as usize)?;
        prism.faces.get(source.face.0 as usize)
    };
    let mut origins = Origins::default();
    for face in &faces {
        let origin = match part_of(*face).ok_or_else(lost)? {
            Part::Near => Origin {
                role: Role::StartCap,
                sources: prism_face(*face).ok_or_else(lost)?.sources.clone(),
            },
            Part::Side => Origin {
                role: Role::Side,
                sources: prism_face(*face).ok_or_else(lost)?.sources.clone(),
            },
            // The end of the profile whose start cap is in the same shell.
            Part::End | Part::Far | Part::Stray => {
                let start = kept
                    .iter()
                    .find(|s| s.faces.contains(face))
                    .and_then(|s| s.faces.iter().find(|f| part_of(**f) == Some(Part::Near)))
                    .and_then(|f| prism_face(*f))
                    .ok_or_else(lost)?;
                Origin {
                    role: Role::EndCap,
                    sources: start.sources.clone(),
                }
            }
        };
        origins.faces.push(origin);
    }

    // Edges and vertices, where each can be told; all or none.
    let near_edge = match near {
        Role::StartCap => Role::StartEdge,
        _ => Role::EndEdge,
    };
    let near_vertex = match near {
        Role::StartCap => Role::StartVertex,
        _ => Role::EndVertex,
    };
    let edge_origin = |edge: EdgeId| -> Option<Origin> {
        match *cut.edges.get(edge.0 as usize)? {
            EdgeSource::Kept {
                operand: Operand::A,
                edge,
            } => {
                let origin = prism.edges.get(edge.0 as usize)?;
                let role = match origin.role {
                    Role::LateralEdge => Role::LateralEdge,
                    role if role == near_edge => Role::StartEdge,
                    _ => return None,
                };
                Some(Origin {
                    role,
                    sources: origin.sources.clone(),
                })
            }
            EdgeSource::Intersection { a, .. } => {
                let origin = prism.faces.get(a.0 as usize)?;
                (origin.role == Role::Side).then(|| Origin {
                    role: Role::EndEdge,
                    sources: origin.sources.clone(),
                })
            }
            EdgeSource::Kept {
                operand: Operand::B,
                ..
            } => None,
        }
    };
    let vertex_origin = |vertex: VertexId| -> Option<Origin> {
        match *cut.vertices.get(vertex.0 as usize)? {
            VertexSource::Kept {
                operand: Operand::A,
                vertex,
            } => {
                let origin = prism.vertices.get(vertex.0 as usize)?;
                (origin.role == near_vertex).then(|| Origin {
                    role: Role::StartVertex,
                    sources: origin.sources.clone(),
                })
            }
            VertexSource::New {
                a: Entity::Edge(edge),
                ..
            } => {
                let origin = prism.edges.get(edge.0 as usize)?;
                (origin.role == Role::LateralEdge).then(|| Origin {
                    role: Role::EndVertex,
                    sources: origin.sources.clone(),
                })
            }
            // On the end edge of a wall, away from its corners: the one
            // vertex of a round end, as a plain extrusion names it.
            VertexSource::New {
                a: Entity::Face(face),
                ..
            } => {
                let origin = prism.faces.get(face.0 as usize)?;
                let &[curve] = origin.sources.as_slice() else {
                    return None;
                };
                (origin.role == Role::Side).then(|| Origin {
                    role: Role::EndVertex,
                    sources: vec![curve, curve],
                })
            }
            VertexSource::Kept {
                operand: Operand::B,
                ..
            } => None,
        }
    };
    let named_edges: Option<Vec<Origin>> = edges.iter().map(|e| edge_origin(*e)).collect();
    let named_vertices: Option<Vec<Origin>> =
        vertices.iter().map(|v| vertex_origin(*v)).collect();
    if let (Some(e), Some(v)) = (named_edges, named_vertices) {
        origins.edges = e;
        origins.vertices = v;
    }
    Ok(OpResult { body, origins })
}
