//! Thin strips that spiral about a point of a surface's chart (the side of
//! a flat spiral spring, a scroll's floor), for the tests of the loop rules:
//! right faces whose long sides go round a point of the chart many times,
//! each side one spline edge or cut into edges.

#![allow(dead_code)]

use craft3d_geom::{Cone, Curve3, Cylinder, NurbsCurve, Plane, Sphere, Surface};
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, VertexId};
use std::f64::consts::TAU;

/// A spline of `degree` through `f` at `count + 1` places evenly over
/// `(s0, s1)`; with `rational`, fitted with weights that swing between a
/// half and three halves along it (the curve still goes through `f` at the
/// same places).
pub fn through(
    f: &dyn Fn(f64) -> Vec3,
    (s0, s1): (f64, f64),
    count: usize,
    degree: usize,
    rational: bool,
) -> NurbsCurve {
    let params: Vec<f64> = (0..=count)
        .map(|i| s0 + (s1 - s0) * i as f64 / count as f64)
        .collect();
    let weight = |s: f64| {
        if rational {
            1.0 + 0.5 * (7.0 * (s - s0) / (s1 - s0)).psin()
        } else {
            1.0
        }
    };
    let values: Vec<Vec<f64>> = params
        .iter()
        .map(|s| {
            let (p, w) = (f(*s), weight(*s));
            vec![p.x * w, p.y * w, p.z * w, w]
        })
        .collect();
    let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, degree, &values).unwrap();
    let weights: Vec<f64> = ctrl.iter().map(|c| c[3]).collect();
    let points: Vec<Vec3> = ctrl
        .iter()
        .map(|c| Vec3::new(c[0] / c[3], c[1] / c[3], c[2] / c[3]))
        .collect();
    if rational {
        NurbsCurve::new(degree, knots, points, weights).unwrap()
    } else {
        NurbsCurve::polynomial(degree, knots, points).unwrap()
    }
}

/// [`through`], with as many places as it takes to lie within `within` of
/// `f` between them (doubled from `start`; `None` past the most a fit may
/// have).
pub fn fitted(
    f: &dyn Fn(f64) -> Vec3,
    range: (f64, f64),
    start: usize,
    degree: usize,
    rational: bool,
    within: f64,
) -> Option<NurbsCurve> {
    let mut count = start.max(2 * degree + 2);
    loop {
        if count >= craft3d_geom::refine::MAX_INTERPOLATED_BANDED {
            return None;
        }
        let curve = through(f, range, count, degree, rational);
        let mut off = 0.0_f64;
        for i in 0..count {
            for share in [0.25, 0.5, 0.75] {
                let s = range.0 + (range.1 - range.0) * (i as f64 + share) / count as f64;
                off = off.max(curve.eval(s).distance(f(s)));
            }
        }
        if off <= within {
            return Some(curve);
        }
        count *= 2;
    }
}

/// How a spiral strip is made.
#[derive(Clone, Copy, Debug)]
pub struct Strip {
    /// Turns about the point of the chart.
    pub turns: f64,
    /// The chart radii its middle starts and ends at.
    pub radii: (f64, f64),
    /// Its width as a share of the pitch (the gap from one turn to the
    /// next); above 1 the strip overlaps itself.
    pub share: f64,
    /// Edges a turn of the outer and of the inner side (0: the side is one
    /// edge).
    pub cut: (usize, usize),
    pub degree: usize,
    pub rational: bool,
    /// The face turned over, its loop kept (the wrong way round).
    pub flipped: bool,
}

impl Strip {
    pub fn of(turns: f64) -> Strip {
        Strip {
            turns,
            radii: (0.15, 0.55),
            share: 0.35,
            cut: (0, 0),
            degree: 3,
            rational: false,
            flipped: false,
        }
    }

    pub fn width(&self) -> f64 {
        self.share * (self.radii.1 - self.radii.0) / self.turns
    }

    /// The area between its sides in the chart: `∫ r w dθ` over the middle
    /// spiral `r = r0 + c θ`.
    pub fn chart_area(&self) -> f64 {
        let angle = TAU * self.turns;
        let c = (self.radii.1 - self.radii.0) / angle;
        self.width() * (self.radii.0 * angle + 0.5 * c * angle * angle)
    }

    /// The point of the chart at the angle `t` on the side `off` from the
    /// middle spiral.
    pub fn chart(&self, off: f64, t: f64) -> (f64, f64) {
        let c = (self.radii.1 - self.radii.0) / (TAU * self.turns);
        let r = self.radii.0 + c * t + off;
        let (s, cs) = t.psin_cos();
        (r * cs, r * s)
    }
}

/// The strip as a one-face sheet on `surface`, `at` taking chart
/// coordinates to it; `None` where a side cannot be fitted within a
/// twentieth of `tol`. The loop runs counter-clockwise in the chart: out
/// along the outer side, across, back along the inner side, across.
pub fn spiral_strip(
    surface: &Surface,
    at: &dyn Fn(f64, f64) -> Vec3,
    strip: &Strip,
    tol: f64,
) -> Option<Body> {
    let total = TAU * strip.turns;
    let w = strip.width();
    let point = |off: f64, t: f64| {
        let (x, y) = strip.chart(off, t);
        at(x, y)
    };
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [
        point(0.5 * w, 0.0),
        point(0.5 * w, total),
        point(-0.5 * w, total),
        point(-0.5 * w, 0.0),
    ];
    let v: Vec<VertexId> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let side = |body: &mut Body, off: f64, a_turn: usize, from: VertexId, to: VertexId| {
        let pieces = if a_turn == 0 {
            1
        } else {
            ((a_turn as f64 * strip.turns).ceil() as usize).max(1)
        };
        let f = |s: f64| point(off, total * s);
        let mut edges = Vec::new();
        let mut before = from;
        for i in 0..pieces {
            let range = (i as f64 / pieces as f64, (i + 1) as f64 / pieces as f64);
            let start = ((40.0 * strip.turns) as usize / pieces).max(8);
            let curve = fitted(&f, range, start, strip.degree, strip.rational, 0.05 * tol)?;
            let next = if i + 1 == pieces {
                to
            } else {
                body.add_vertex(f(range.1))
            };
            edges.push(body.add_edge(Curve3::Nurbs(curve), range, before, next));
            before = next;
        }
        Some(edges)
    };
    let outer = side(&mut body, 0.5 * w, strip.cut.0, v[0], v[1])?;
    let inner = side(&mut body, -0.5 * w, strip.cut.1, v[3], v[2])?;
    let across = |body: &mut Body, t: f64, from: VertexId, to: VertexId, inwards: bool| {
        let f = |s: f64| {
            let off = if inwards {
                0.5 * w - w * s
            } else {
                -0.5 * w + w * s
            };
            point(off, t)
        };
        let curve = fitted(&f, (0.0, 1.0), 8, 3, false, 0.05 * tol)?;
        Some(body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), from, to))
    };
    let end = across(&mut body, total, v[1], v[2], true)?;
    let start = across(&mut body, 0.0, v[3], v[0], false)?;
    let forward = |edge| Coedge {
        edge,
        reversed: false,
    };
    let mut coedges: Vec<Coedge> = outer.iter().map(|e| forward(*e)).collect();
    coedges.push(forward(end));
    coedges.extend(inner.iter().rev().map(|e| Coedge {
        edge: *e,
        reversed: true,
    }));
    coedges.push(forward(start));
    let lp = body.add_loop(coedges);
    let face = body.add_face(surface.clone(), strip.flipped, vec![lp]);
    body.add_shell(vec![face]);
    Some(body)
}

/// A surface of size `k` with the map from the chart coordinates a spiral
/// is drawn in (within a unit of the chart's origin) to it, and the factor
/// from the chart's area to the surface's where the map is that simple (a
/// plane, a cylinder).
pub type Ground = (
    &'static str,
    Surface,
    Box<dyn Fn(f64, f64) -> Vec3>,
    Option<f64>,
);

pub fn grounds(k: f64) -> Vec<Ground> {
    let slope = 0.4_f64.ptan();
    vec![
        (
            "plane",
            Surface::Plane(Plane::new(Frame::WORLD)),
            Box::new(move |x, y| Vec3::new(3.0 * k * x, 3.0 * k * y, 0.0)),
            Some(9.0 * k * k),
        ),
        (
            "cylinder",
            Surface::Cylinder(Cylinder::new(Frame::WORLD, k).unwrap()),
            Box::new(move |x, y| {
                let (s, c) = x.psin_cos();
                Vec3::new(k * c, k * s, k * y)
            }),
            Some(k * k),
        ),
        (
            "cone",
            Surface::Cone(Cone::new(Frame::WORLD, 2.0 * k, 0.4).unwrap()),
            Box::new(move |x, y| {
                let (s, c) = x.psin_cos();
                let r = 2.0 * k + k * y * slope;
                Vec3::new(r * c, r * s, k * y)
            }),
            None,
        ),
        (
            "sphere",
            Surface::Sphere(Sphere::new(Frame::WORLD, 2.0 * k).unwrap()),
            Box::new(move |x, y| {
                let (su, cu) = x.psin_cos();
                let (sv, cv) = y.psin_cos();
                Vec3::new(2.0 * k * cv * cu, 2.0 * k * cv * su, 2.0 * k * sv)
            }),
            None,
        ),
    ]
}

/// More grounds for a spiral: the outer half of a torus, and a Bézier
/// patch that is no surface of revolution (a saddle with a bulge, its
/// parameters running unevenly over it).
pub fn more_grounds(k: f64) -> Vec<Ground> {
    let (major, minor) = (3.0 * k, k);
    let mut ctrl = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            // Rows and columns that are not evenly spaced, so the chart
            // does not run evenly with the surface.
            let x = [0.0, 0.2, 0.55, 1.0][i] * 4.0 * k;
            let y = [0.0, 0.45, 0.8, 1.0][j] * 3.0 * k;
            let saddle = 0.6 * (i as f64 - 1.5) * (j as f64 - 1.5) / 2.25;
            let bulge = if i == 1 && j == 2 { 0.8 } else { 0.0 };
            ctrl.push(Vec3::new(x, y, k * (saddle + bulge)));
        }
    }
    let patch = Surface::Bezier(craft3d_geom::BezierPatch::new(3, 3, ctrl).unwrap());
    let on_patch = patch.clone();
    vec![
        (
            "torus",
            Surface::Torus(craft3d_geom::Torus::new(Frame::WORLD, major, minor).unwrap()),
            Box::new(move |x, y| {
                let (su, cu) = (0.5 * x).psin_cos();
                let (sv, cv) = y.psin_cos();
                let r = major + minor * cv;
                Vec3::new(r * cu, r * su, minor * sv)
            }),
            None,
        ),
        (
            "patch",
            patch,
            Box::new(move |x, y| on_patch.eval(0.5 + 0.7 * x, 0.5 + 0.7 * y)),
            None,
        ),
    ]
}

/// A tolerance for a body of size `k`.
pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Turned, and `far` sizes `k` off.
pub fn turned(k: f64, far: f64) -> Transform {
    Transform::from_frame(
        &Frame::new(
            Vec3::new(1.3, -2.1, 0.7) * (k * far),
            Vec3::new(-0.3, 0.8, 0.52),
            Vec3::new(0.6, 0.1, 0.4),
        )
        .unwrap(),
    )
}
