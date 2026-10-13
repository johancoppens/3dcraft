//! Shared by the tests of rational freeform faces: a body with faces on
//! cylinders, cones, spheres and tori, and the *same* body with each of
//! those faces on the rational NURBS surface that is that piece of the
//! quadric or torus exactly (the standard rational quadratic arcs, *The
//! NURBS Book* §7.3 and §8.5: an arc of angle `d` has the control points of
//! its ends with weight 1 and the meeting point of their tangents with
//! weight `cos(d / 2)`; a surface of revolution is the product of such an
//! arc and its profile). The analytic body is the oracle for the rational
//! one: they are the same solid.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::Body;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A rational curve in a plane: degree, knots, points, weights.
#[derive(Clone, Debug)]
pub struct Flat {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub points: Vec<(f64, f64)>,
    pub weights: Vec<f64>,
}

/// The arc of the unit circle from the angle `from` to `to` (`to` above
/// `from`), in pieces of a quarter turn at most, with its parameter
/// running over `[start, end]`: as wide as one likes, because a bound
/// that forgets the width of a span is a wrong bound.
pub fn arc(from: f64, to: f64, start: f64, end: f64) -> Flat {
    let spans = (((to - from) / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
    let step = (to - from) / spans as f64;
    let weight = (0.5 * step).cos();
    let mut points = vec![(from.cos(), from.sin())];
    let mut weights = vec![1.0];
    let mut knots = vec![start; 3];
    for s in 0..spans {
        let middle = from + step * (s as f64 + 0.5);
        let last = from + step * (s as f64 + 1.0);
        points.push((middle.cos() / weight, middle.sin() / weight));
        weights.push(weight);
        points.push((last.cos(), last.sin()));
        weights.push(1.0);
        let knot = if s + 1 == spans {
            end
        } else {
            start + (end - start) * (s as f64 + 1.0) / spans as f64
        };
        knots.push(knot);
        knots.push(knot);
    }
    knots.push(end);
    Flat {
        degree: 2,
        knots,
        points,
        weights,
    }
}

/// The straight profile between two points.
pub fn straight(a: (f64, f64), b: (f64, f64)) -> Flat {
    Flat {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![a, b],
        weights: vec![1.0, 1.0],
    }
}

/// The surface swept by `profile` (distance from the axis, height) about
/// the z axis of `frame` from the angle `from` to `to`. u goes around over
/// `[0, 3]` (not one wide, on purpose), v along the profile.
pub fn revolved(frame: &Frame, profile: &Flat, from: f64, to: f64) -> NurbsSurface {
    let around = arc(from, to, 0.0, 3.0);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (&(x, y), &wu) in around.points.iter().zip(&around.weights) {
        for (&(reach, height), &wv) in profile.points.iter().zip(&profile.weights) {
            ctrl.push(frame.point(reach * x, reach * y, height));
            weights.push(wu * wv);
        }
    }
    NurbsSurface::new(
        2,
        profile.degree,
        around.knots,
        profile.knots.clone(),
        around.points.len(),
        profile.points.len(),
        ctrl,
        weights,
    )
    .unwrap()
}

/// The lowest and highest of some angles that lie within half a turn of
/// the first, unwrapped around it.
fn angle_range(angles: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut range: Option<(f64, f64, f64)> = None;
    for a in angles {
        match &mut range {
            None => range = Some((a, a, a)),
            Some((first, lo, hi)) => {
                let a = *first + (a - *first + PI).rem_euclid(TAU) - PI;
                *lo = lo.min(a);
                *hi = hi.max(a);
            }
        }
    }
    let (_, lo, hi) = range.unwrap();
    (lo, hi)
}

fn range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
        (lo.min(x), hi.max(x))
    })
}

fn padded((lo, hi): (f64, f64), pad: f64) -> (f64, f64) {
    let more = pad * (hi - lo);
    (lo - more, hi + more)
}

/// The rational surface that is the part of `surface` (a cylinder, cone,
/// sphere or torus) the points lie on, and `pad` times its extent more on
/// every side (with no padding the outermost points are on the border of
/// the patch). `None` for another kind of surface.
pub fn rational_part(surface: &Surface, points: &[Vec3], pad: f64) -> Option<NurbsSurface> {
    let frame = match surface {
        Surface::Cylinder(s) => *s.frame(),
        Surface::Cone(s) => *s.frame(),
        Surface::Sphere(s) => *s.frame(),
        Surface::Torus(s) => *s.frame(),
        _ => return None,
    };
    let local: Vec<(f64, f64, f64)> = points.iter().map(|&p| frame.local(p)).collect();
    let (from, to) = padded(angle_range(local.iter().map(|p| p.1.atan2(p.0))), pad);
    // (The angles are unwrapped about the first: a face that reaches
    // further around, or around a pole, would get a patch that is not
    // under all of it.)
    assert!(to - from < PI, "the face goes more than half way around");
    let heights = || padded(range(local.iter().map(|p| p.2)), pad);
    let profile = match surface {
        Surface::Cylinder(s) => {
            let (low, high) = heights();
            straight((s.radius(), low), (s.radius(), high))
        }
        Surface::Cone(s) => {
            let (low, high) = heights();
            let slope = s.half_angle().tan();
            let reach = |z: f64| s.radius() + z * slope;
            assert!(reach(low) > 0.0 && reach(high) > 0.0, "through the apex");
            straight((reach(low), low), (reach(high), high))
        }
        Surface::Sphere(s) => {
            let (low, high) = padded(range(local.iter().map(|p| p.2.atan2(p.0.hypot(p.1)))), pad);
            // Up to a pole the patch is still the sphere, with a border
            // collapsed into the pole.
            let (low, high) = (low.max(-FRAC_PI_2), high.min(FRAC_PI_2));
            let mut profile = arc(low, high, 0.0, 1.0);
            for p in &mut profile.points {
                *p = (p.0 * s.radius(), p.1 * s.radius());
            }
            profile
        }
        Surface::Torus(s) => {
            let (major, minor) = s.radii();
            let (low, high) = padded(
                angle_range(local.iter().map(|p| p.2.atan2(p.0.hypot(p.1) - major))),
                pad,
            );
            assert!(
                high - low < PI,
                "the face goes more than half way around the tube"
            );
            let mut profile = arc(low, high, 0.0, 2.0);
            for p in &mut profile.points {
                *p = (major + p.0 * minor, p.1 * minor);
            }
            profile
        }
        _ => return None,
    };
    Some(revolved(&frame, &profile, from, to))
}

/// The circle arc of an edge as a rational curve over the same range of
/// parameter, with the same ends.
fn rational_arc(curve: &Curve3, range: (f64, f64)) -> Option<Curve3> {
    let Curve3::Circle(circle) = curve else {
        return None;
    };
    let flat = arc(range.0, range.1, range.0, range.1);
    let (frame, radius) = (circle.frame(), circle.radius());
    let ctrl = flat
        .points
        .iter()
        .map(|&(x, y)| frame.point(x * radius, y * radius, 0.0))
        .collect();
    Some(Curve3::Nurbs(
        NurbsCurve::new(2, flat.knots, ctrl, flat.weights).unwrap(),
    ))
}

/// The same body with every face on a cylinder, cone, sphere or torus put
/// on the rational surface that is that piece of it (see
/// [`rational_part`]); with `edges`, every circular edge becomes the
/// rational arc it is as well. How many faces were replaced comes along.
pub fn rationalized(body: &Body, pad: f64, edges: bool) -> (Body, usize) {
    let mut twin = body.clone();
    let mut replaced = 0;
    for (index, face) in body.faces.iter().enumerate() {
        let mut points = Vec::new();
        for &loop_id in &face.loops {
            for coedge in &body.loop_(loop_id).unwrap().coedges {
                let edge = body.edge(coedge.edge).unwrap();
                for k in 0..=16 {
                    let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(k) / 16.0;
                    points.push(edge.curve.eval(t));
                }
            }
        }
        let Some(patch) = rational_part(&face.surface, &points, pad) else {
            continue;
        };
        let patch = Surface::Nurbs(patch);
        // The patch's normal may be the other way round than the
        // surface's at the same place: the face keeps its outside.
        let ((u0, u1), (v0, v1)) = patch.domain();
        let (u, v) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
        let at = patch.eval(u, v);
        let (su, sv) = face.surface.project(at).unwrap();
        assert!(
            face.surface.eval(su, sv).distance(at) <= 1e-9 * (1.0 + at.length()),
            "the patch is not on the surface"
        );
        let same = patch
            .normal(u, v)
            .unwrap()
            .dot(face.surface.normal(su, sv).unwrap())
            > 0.0;
        twin.faces[index].surface = patch;
        twin.faces[index].reversed = face.reversed == same;
        replaced += 1;
    }
    if edges {
        for edge in &mut twin.edges {
            if let Some(curve) = rational_arc(&edge.curve, edge.range) {
                edge.curve = curve;
            }
        }
    }
    (twin, replaced)
}
