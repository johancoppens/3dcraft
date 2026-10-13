//! Bodies for the STEP tests that the reference solids of the test kit do
//! not cover: cavities, several lumps, rational splines, a sheet.

use craft3d_geom::{Circle, Curve3, Ellipse, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, merge};
use std::f64::consts::TAU;

/// The same faces seen from the other side: a solid turned inside out,
/// which is what a cavity is.
pub fn inside_out(body: &Body) -> Body {
    let mut out = body.clone();
    for face in &mut out.faces {
        face.reversed = !face.reversed;
    }
    for loop_ in &mut out.loops {
        loop_.coedges.reverse();
        for coedge in &mut loop_.coedges {
            coedge.reversed = !coedge.reversed;
        }
    }
    out
}

fn at(x: f64, y: f64, z: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z)).unwrap()
}

/// A cube of side `4 s` with a spherical cavity of radius `s` in its
/// middle: two shells, one of them a face without loops.
pub fn block_with_spherical_cavity(s: f64) -> Body {
    let block = primitives::block(&Frame::WORLD, 4.0 * s, 4.0 * s, 4.0 * s).unwrap();
    let cavity = inside_out(&primitives::sphere(&at(2.0 * s, 2.0 * s, 2.0 * s), s).unwrap());
    merge(&[&block, &cavity]).unwrap().0
}

/// A cube of side `4 s` with a box-shaped cavity and a conical one: the
/// cavities have edges, loops and an apex.
pub fn block_with_two_cavities(s: f64) -> Body {
    let block = primitives::block(&Frame::WORLD, 4.0 * s, 4.0 * s, 4.0 * s).unwrap();
    let hollow = inside_out(
        &primitives::block(&at(0.5 * s, 0.5 * s, 0.5 * s), s, 1.5 * s, 2.0 * s).unwrap(),
    );
    let cone =
        inside_out(&primitives::cone(&at(3.0 * s, 3.0 * s, 1.0 * s), 0.5 * s, 2.0 * s).unwrap());
    merge(&[&block, &hollow, &cone]).unwrap().0
}

/// A block and a sphere apart from each other in one body: two lumps.
pub fn two_lumps(s: f64) -> Body {
    let block = primitives::block(&Frame::WORLD, 2.0 * s, 2.0 * s, 2.0 * s).unwrap();
    let ball = primitives::sphere(&at(6.0 * s, 0.0, 0.0), s).unwrap();
    merge(&[&block, &ball]).unwrap().0
}

/// A cube of side 10 with a cubic cavity of side 6, in which floats a cube
/// of side 4 with a spherical cavity of radius 1, and next to the big cube
/// a cylinder: three lumps and two cavities, each cavity in its own lump.
/// The shells are deliberately not in order of nesting.
pub fn nested(s: f64) -> Body {
    let inner_cavity = inside_out(&primitives::sphere(&at(5.0 * s, 5.0 * s, 5.0 * s), s).unwrap());
    let outer = primitives::block(&Frame::WORLD, 10.0 * s, 10.0 * s, 10.0 * s).unwrap();
    let floating =
        primitives::block(&at(3.0 * s, 3.0 * s, 3.0 * s), 4.0 * s, 4.0 * s, 4.0 * s).unwrap();
    let cavity = inside_out(
        &primitives::block(&at(2.0 * s, 2.0 * s, 2.0 * s), 6.0 * s, 6.0 * s, 6.0 * s).unwrap(),
    );
    let beside = primitives::cylinder(&at(15.0 * s, 0.0, 0.0), s, 3.0 * s).unwrap();
    merge(&[&inner_cavity, &floating, &beside, &cavity, &outer])
        .unwrap()
        .0
}

/// A cylinder of radius `r` and height `h` on the frame whose wall is a
/// rational B-spline surface and whose circles are rational B-spline
/// curves: the wall has a seam, used twice by its one loop.
pub fn nurbs_can(frame: &Frame, r: f64, h: f64) -> Body {
    let top_frame = frame.at(frame.point(0.0, 0.0, h)).unwrap();
    let bottom_curve = NurbsCurve::circle(frame, r).unwrap();
    let top_curve = NurbsCurve::circle(&top_frame, r).unwrap();
    // u along the circle, v up: two rows of v per control point of u.
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (i, (&low, &high)) in bottom_curve
        .control_points()
        .iter()
        .zip(top_curve.control_points())
        .enumerate()
    {
        ctrl.extend([low, high]);
        weights.extend([bottom_curve.weights()[i]; 2]);
    }
    let wall_surface = NurbsSurface::new(
        2,
        1,
        bottom_curve.knots().to_vec(),
        vec![0.0, 0.0, 1.0, 1.0],
        9,
        2,
        ctrl,
        weights,
    )
    .unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let low = body.add_vertex(frame.point(r, 0.0, 0.0));
    let high = body.add_vertex(frame.point(r, 0.0, h));
    let bottom = body.add_edge(Curve3::Nurbs(bottom_curve), (0.0, 1.0), low, low);
    let top = body.add_edge(Curve3::Nurbs(top_curve), (0.0, 1.0), high, high);
    let seam = body.add_edge(
        Curve3::Line(Line::new(frame.point(r, 0.0, 0.0), frame.z()).unwrap()),
        (0.0, h),
        low,
        high,
    );
    let use_ = |edge, reversed| Coedge { edge, reversed };
    let wall_loop = body.add_loop(vec![
        use_(bottom, false),
        use_(seam, false),
        use_(top, true),
        use_(seam, true),
    ]);
    let wall = body.add_face(Surface::Nurbs(wall_surface), false, vec![wall_loop]);
    let floor_loop = body.add_loop(vec![use_(bottom, true)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(frame.reversed_at(frame.origin()).unwrap())),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![use_(top, false)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    body
}

/// An elliptic cylinder with semi-axes `a` along x and `b` along y: its
/// rims are whole ellipses. With `a < b` the longer axis is the second.
/// The wall is a ruled B-spline surface through the rational ellipse.
pub fn elliptic_disc(frame: &Frame, a: f64, b: f64) -> Body {
    // A flat sheet: one planar face bounded by an ellipse.
    let mut body = Body::new(BodyKind::Sheet);
    let vertex = body.add_vertex(frame.point(a, 0.0, 0.0));
    let edge = body.add_edge(
        Curve3::Ellipse(Ellipse::new(*frame, a, b).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let loop_ = body.add_loop(vec![Coedge {
        edge,
        reversed: false,
    }]);
    let face = body.add_face(Surface::Plane(Plane::new(*frame)), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

/// A block without its lid: five faces, a sheet.
pub fn open_box(s: f64) -> Body {
    let mut body = primitives::block(&Frame::WORLD, 3.0 * s, 2.0 * s, s).unwrap();
    body.kind = BodyKind::Sheet;
    // Face 1 is +z (see `primitives::block`); its loop goes with it.
    let lid = body.faces.remove(1);
    let lid_loop = lid.loops[0].0 as usize;
    body.loops.remove(lid_loop);
    for face in &mut body.faces {
        for loop_ in &mut face.loops {
            if loop_.0 as usize > lid_loop {
                loop_.0 -= 1;
            }
        }
    }
    body.shells[0].faces = (0..5).map(craft3d_topo::FaceId).collect();
    body
}

/// A disc with a circular hole: a planar sheet face with two loops.
pub fn washer(frame: &Frame, outer: f64, inner: f64) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let mut circle = |radius: f64| {
        let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
        body.add_edge(
            Curve3::Circle(Circle::new(*frame, radius).unwrap()),
            (0.0, TAU),
            vertex,
            vertex,
        )
    };
    let (rim, hole) = (circle(outer), circle(inner));
    // The hole first: the export must still call the rim the outer bound.
    let hole_loop = body.add_loop(vec![Coedge {
        edge: hole,
        reversed: true,
    }]);
    let rim_loop = body.add_loop(vec![Coedge {
        edge: rim,
        reversed: false,
    }]);
    let face = body.add_face(
        Surface::Plane(Plane::new(*frame)),
        false,
        vec![hole_loop, rim_loop],
    );
    body.add_shell(vec![face]);
    body
}

/// A quarter of a cylinder of radius `r` and height `h` in the first
/// quadrant of the frame, whose curved wall is a rational B-spline
/// surface of one span and whose arcs are rational B-spline curves:
/// volume `π r² h / 4`, area `π r² / 2 + 2 r h + π r h / 2`.
pub fn quarter_round(frame: &Frame, r: f64, h: f64) -> Body {
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let p = |x: f64, y: f64, z: f64| frame.point(x, y, z);
    let arc = |z: f64| {
        NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![p(r, 0.0, z), p(r, r, z), p(0.0, r, z)],
            vec![1.0, w, 1.0],
        )
        .unwrap()
    };
    let wall_surface = NurbsSurface::new(
        2,
        1,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        3,
        2,
        vec![
            p(r, 0.0, 0.0),
            p(r, 0.0, h),
            p(r, r, 0.0),
            p(r, r, h),
            p(0.0, r, 0.0),
            p(0.0, r, h),
        ],
        vec![1.0, 1.0, w, w, 1.0, 1.0],
    )
    .unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let [o0, a0, b0, o1, a1, b1] = [
        p(0.0, 0.0, 0.0),
        p(r, 0.0, 0.0),
        p(0.0, r, 0.0),
        p(0.0, 0.0, h),
        p(r, 0.0, h),
        p(0.0, r, h),
    ]
    .map(|point| body.add_vertex(point));
    let mut line = |from: craft3d_topo::VertexId, to: craft3d_topo::VertexId| {
        let (start, end) = (
            body.vertex(from).unwrap().point,
            body.vertex(to).unwrap().point,
        );
        body.add_edge(
            Curve3::Line(Line::through(start, end).unwrap()),
            (0.0, start.distance(end)),
            from,
            to,
        )
    };
    let (oa0, bo0, oa1, bo1) = (line(o0, a0), line(b0, o0), line(o1, a1), line(b1, o1));
    let (va, vb, vo) = (line(a0, a1), line(b0, b1), line(o0, o1));
    let arc0 = body.add_edge(Curve3::Nurbs(arc(0.0)), (0.0, 1.0), a0, b0);
    let arc1 = body.add_edge(Curve3::Nurbs(arc(h)), (0.0, 1.0), a1, b1);
    let f = |edge| Coedge {
        edge,
        reversed: false,
    };
    let b = |edge| Coedge {
        edge,
        reversed: true,
    };
    let plane = |origin: Vec3, z: Vec3, x: Vec3| {
        Surface::Plane(Plane::new(Frame::new(origin, z, x).unwrap()))
    };
    let (x, y, z) = (frame.x(), frame.y(), frame.z());
    let origin = frame.origin();
    let top = p(0.0, 0.0, h);
    let mut faces = Vec::new();
    let l = body.add_loop(vec![f(arc0), f(vb), b(arc1), b(va)]);
    faces.push(body.add_face(Surface::Nurbs(wall_surface), false, vec![l]));
    let l = body.add_loop(vec![b(bo0), b(arc0), b(oa0)]);
    faces.push(body.add_face(plane(origin, -z, x), false, vec![l]));
    let l = body.add_loop(vec![f(oa1), f(arc1), f(bo1)]);
    faces.push(body.add_face(plane(top, z, x), false, vec![l]));
    let l = body.add_loop(vec![f(oa0), f(va), b(oa1), b(vo)]);
    faces.push(body.add_face(plane(origin, -y, x), false, vec![l]));
    let l = body.add_loop(vec![f(bo0), f(vo), b(bo1), b(vb)]);
    faces.push(body.add_face(plane(origin, -x, y), false, vec![l]));
    body.add_shell(faces);
    body
}
