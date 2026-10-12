//! Primitive solids, built directly as B-rep.

use crate::KernelError;
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, GeomError, Line, Plane, Sphere, Surface, Torus,
};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

fn invalid(message: &'static str) -> GeomError {
    GeomError::InvalidParameter(message)
}

/// Sizes must be positive and finite; says which one is not.
fn size(name: &'static str, value: f64) -> Result<f64, KernelError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(KernelError::InvalidSize { name, value })
    }
}

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

/// A frame at `origin` looking the other way: its z axis is `-frame.z`.
fn flipped(frame: &Frame, origin: Vec3) -> Result<Frame, GeomError> {
    Frame::new(origin, -frame.z(), frame.x()).ok_or(invalid("degenerate frame"))
}

fn lifted(frame: &Frame, height: f64) -> Result<Frame, GeomError> {
    Frame::new(frame.point(0.0, 0.0, height), frame.z(), frame.x())
        .ok_or(invalid("degenerate frame"))
}

/// A full circle of `radius` about the z axis of `frame`, with its one
/// vertex on the x axis.
fn full_circle(body: &mut Body, frame: Frame, radius: f64) -> Result<EdgeId, GeomError> {
    let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let circle = Curve3::Circle(Circle::new(frame, radius)?);
    Ok(body.add_edge(circle, (0.0, TAU), vertex, vertex))
}

/// A rectangular block with one corner at the frame's origin and edges
/// `dx`, `dy`, `dz` along its axes: 8 vertices, 12 edges, 6 planar faces.
pub fn block(frame: &Frame, dx: f64, dy: f64, dz: f64) -> Result<Body, KernelError> {
    let (dx, dy, dz) = (size("dx", dx)?, size("dy", dy)?, size("dz", dz)?);
    let mut body = Body::new(BodyKind::Solid);
    let corner = |i: u32, j: u32, k: u32| {
        frame.point(f64::from(i) * dx, f64::from(j) * dy, f64::from(k) * dz)
    };
    let mut vertices = Vec::new();
    for index in 0..8_u32 {
        vertices.push(body.add_vertex(corner(index & 1, (index >> 1) & 1, (index >> 2) & 1)));
    }
    // Corner indices (i + 2j + 4k) per face, counter-clockwise from outside.
    let faces: [[usize; 4]; 6] = [
        [0, 2, 3, 1], // -z
        [4, 5, 7, 6], // +z
        [0, 1, 5, 4], // -y
        [2, 6, 7, 3], // +y
        [0, 4, 6, 2], // -x
        [1, 3, 7, 5], // +x
    ];
    let mut edges: BTreeMap<(VertexId, VertexId), EdgeId> = BTreeMap::new();
    let mut face_ids = Vec::new();
    for quad in faces {
        let ids: Vec<VertexId> = quad
            .iter()
            .filter_map(|&c| vertices.get(c).copied())
            .collect();
        let points: Vec<Vec3> = ids
            .iter()
            .filter_map(|&v| body.vertex(v).map(|x| x.point))
            .collect();
        let (&[a, b, c, d], &[pa, pb, _, pd]) = (ids.as_slice(), points.as_slice()) else {
            return Err(KernelError::from(invalid(
                "block corner table is inconsistent",
            )));
        };
        let mut coedges = Vec::new();
        for (from, to) in [(a, b), (b, c), (c, d), (d, a)] {
            let key = (from.min(to), from.max(to));
            let edge = match edges.get(&key) {
                Some(&edge) => edge,
                None => {
                    let (Some(start), Some(end)) = (body.vertex(key.0), body.vertex(key.1)) else {
                        return Err(KernelError::from(invalid(
                            "block corner table is inconsistent",
                        )));
                    };
                    let (start, end) = (start.point, end.point);
                    let line = Curve3::Line(Line::through(start, end)?);
                    let edge = body.add_edge(line, (0.0, start.distance(end)), key.0, key.1);
                    edges.insert(key, edge);
                    edge
                }
            };
            coedges.push(Coedge {
                edge,
                reversed: from > to,
            });
        }
        let normal = (pb - pa).cross(pd - pa);
        let plane = Frame::new(pa, normal, pb - pa).ok_or(invalid("block has a zero-size face"))?;
        let loop_ = body.add_loop(coedges);
        face_ids.push(body.add_face(Surface::Plane(Plane::new(plane)), false, vec![loop_]));
    }
    body.add_shell(face_ids);
    Ok(body)
}

/// A cylinder standing on the frame's xy plane, about its z axis: a wall
/// between two full circles, a floor and a lid.
pub fn cylinder(frame: &Frame, radius: f64, height: f64) -> Result<Body, KernelError> {
    let (radius, height) = (size("radius", radius)?, size("height", height)?);
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = lifted(frame, height)?;
    let bottom = full_circle(&mut body, *frame, radius)?;
    let top = full_circle(&mut body, top_frame, radius)?;
    // Seen from outside the wall lies above the bottom circle and below
    // the top one: left of the bottom circle running forward, and of the
    // top one running backward.
    let wall_loops = vec![
        body.add_loop(vec![forward(bottom)]),
        body.add_loop(vec![backward(top)]),
    ];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        wall_loops,
    );
    let floor_loop = body.add_loop(vec![backward(bottom)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(flipped(frame, frame.origin())?)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![forward(top)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    Ok(body)
}

/// A cone standing on the frame's xy plane with its apex at `height` on
/// the z axis.
pub fn cone(frame: &Frame, radius: f64, height: f64) -> Result<Body, KernelError> {
    let (radius, height) = (size("radius", radius)?, size("height", height)?);
    let mut body = Body::new(BodyKind::Solid);
    let base = full_circle(&mut body, *frame, radius)?;
    // Narrowing along +z: a negative half-angle.
    let surface = Surface::Cone(Cone::new(*frame, radius, -(radius / height).atan())?);
    let side_loop = body.add_loop(vec![forward(base)]);
    let side = body.add_face(surface, false, vec![side_loop]);
    let floor_loop = body.add_loop(vec![backward(base)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(flipped(frame, frame.origin())?)),
        false,
        vec![floor_loop],
    );
    body.add_shell(vec![side, floor]);
    Ok(body)
}

/// A sphere about the frame's origin: one face, no edges.
pub fn sphere(frame: &Frame, radius: f64) -> Result<Body, KernelError> {
    let radius = size("radius", radius)?;
    let mut body = Body::new(BodyKind::Solid);
    let face = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius)?),
        false,
        Vec::new(),
    );
    body.add_shell(vec![face]);
    Ok(body)
}

/// A ring torus about the frame's z axis: one face, no edges, one handle.
pub fn torus(frame: &Frame, major: f64, minor: f64) -> Result<Body, KernelError> {
    let (major, minor) = (size("major", major)?, size("minor", minor)?);
    let mut body = Body::new(BodyKind::Solid);
    let face = body.add_face(
        Surface::Torus(Torus::new(*frame, major, minor)?),
        false,
        Vec::new(),
    );
    body.add_shell(vec![face]);
    Ok(body)
}
