//! Reference solids with trimmed faces, built by hand, with their
//! closed-form mass properties (oracle 1 in docs/testing.md): faces with
//! holes, faces that are part of a cylinder, cone, sphere or torus, an
//! ellipse edge, spline edges and a freeform face. Each is placed by a
//! frame and sized by its arguments, so tests can tilt and scale it.
//!
//! Before any of them is used as an oracle, `craft3d-tess`'s
//! `tests/fixtures.rs` proves it: `craft3d_topo::check` finds no defect
//! and `craft3d_topo::mass_properties` agrees with the closed forms.

use crate::solids::Reference;
use craft3d_geom::{
    BezierPatch, Circle, Cone, Curve3, Cylinder, Ellipse, GeomError, Line, NurbsCurve,
    NurbsSurface, Plane, Sphere, Surface, Torus,
};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId, mass_properties};
use std::f64::consts::{PI, TAU};

fn invalid(message: &'static str) -> GeomError {
    GeomError::InvalidParameter(message)
}

fn positive(values: &[f64]) -> Result<(), GeomError> {
    if values.iter().all(|v| v.is_finite() && *v > 0.0) {
        Ok(())
    } else {
        Err(invalid("sizes must be positive and finite"))
    }
}

fn frame_at(origin: Vec3, z: Vec3, x: Vec3) -> Result<Frame, GeomError> {
    Frame::new(origin, z, x).ok_or(invalid("degenerate frame"))
}

fn coedge(edge: EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

/// A full circle of `radius` about the z axis of `frame`, with its one
/// vertex on the x axis.
fn full_circle(body: &mut Body, frame: Frame, radius: f64) -> Result<EdgeId, GeomError> {
    let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let circle = Curve3::Circle(Circle::new(frame, radius)?);
    Ok(body.add_edge(circle, (0.0, TAU), vertex, vertex))
}

/// How a profile gets from one corner to the next.
#[derive(Clone, Copy)]
enum Side {
    Line,
    /// Along a circle about `centre`, counter-clockwise seen from +z
    /// unless `clockwise`.
    Arc {
        centre: (f64, f64),
        radius: f64,
        clockwise: bool,
    },
}

/// A closed outline in the xy plane: side `i` runs from corner `i` to
/// corner `i + 1`. The material is to its left: outer outlines run
/// counter-clockwise, holes clockwise. One corner with one arc is a full
/// circle.
struct Profile {
    corners: Vec<(f64, f64)>,
    sides: Vec<Side>,
}

fn rectangle(x: (f64, f64), y: (f64, f64), clockwise: bool) -> Profile {
    let mut corners = vec![(x.0, y.0), (x.1, y.0), (x.1, y.1), (x.0, y.1)];
    if clockwise {
        corners.reverse();
    }
    Profile {
        corners,
        sides: vec![Side::Line; 4],
    }
}

/// One outline at one height: its vertices and edges, and for each edge
/// whether it runs against the outline.
struct Layer {
    vertices: Vec<VertexId>,
    edges: Vec<(EdgeId, bool)>,
}

fn layer(body: &mut Body, frame: &Frame, profile: &Profile, z: f64) -> Result<Layer, GeomError> {
    let n = profile.corners.len();
    let vertices: Vec<VertexId> = profile
        .corners
        .iter()
        .map(|&(x, y)| body.add_vertex(frame.point(x, y, z)))
        .collect();
    let mut edges = Vec::with_capacity(n);
    for (i, side) in profile.sides.iter().enumerate() {
        let j = (i + 1) % n;
        let (Some(&from), Some(&to), Some(&p), Some(&q)) = (
            vertices.get(i),
            vertices.get(j),
            profile.corners.get(i),
            profile.corners.get(j),
        ) else {
            return Err(invalid("a profile needs as many sides as corners"));
        };
        match *side {
            Side::Line => {
                let (start, end) = (frame.point(p.0, p.1, z), frame.point(q.0, q.1, z));
                let line = Curve3::Line(Line::through(start, end)?);
                edges.push((
                    body.add_edge(line, (0.0, start.distance(end)), from, to),
                    false,
                ));
            }
            Side::Arc {
                centre,
                radius,
                clockwise,
            } => {
                // The circle runs counter-clockwise; a clockwise side
                // uses it backwards.
                let (first, last, start, end) = if clockwise {
                    (q, p, to, from)
                } else {
                    (p, q, from, to)
                };
                let angle = |c: (f64, f64)| (c.1 - centre.1).atan2(c.0 - centre.0);
                let begin = angle(first);
                let mut sweep = (angle(last) - begin).rem_euclid(TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                let circle_frame =
                    frame_at(frame.point(centre.0, centre.1, z), frame.z(), frame.x())?;
                let circle = Curve3::Circle(Circle::new(circle_frame, radius)?);
                edges.push((
                    body.add_edge(circle, (begin, begin + sweep), start, end),
                    clockwise,
                ));
            }
        }
    }
    Ok(Layer { vertices, edges })
}

/// The solid swept by `profiles` (the first the outer outline, the others
/// holes) from the frame's xy plane up to `height`.
fn extrude(frame: &Frame, profiles: &[Profile], height: f64) -> Result<Body, GeomError> {
    let mut body = Body::new(BodyKind::Solid);
    let mut faces = Vec::new();
    let mut floor_loops = Vec::new();
    let mut lid_loops = Vec::new();
    for profile in profiles {
        let n = profile.corners.len();
        if n == 0 || profile.sides.len() != n {
            return Err(invalid("a profile needs as many sides as corners"));
        }
        let bottom = layer(&mut body, frame, profile, 0.0)?;
        let top = layer(&mut body, frame, profile, height)?;
        // One upright edge per corner; none for a full circle, whose
        // wall closes on itself.
        let mut uprights = Vec::new();
        if n > 1 {
            for (&from, &to) in bottom.vertices.iter().zip(&top.vertices) {
                let (Some(a), Some(b)) = (body.vertex(from), body.vertex(to)) else {
                    return Err(invalid("a profile corner is missing"));
                };
                let (a, b) = (a.point, b.point);
                let line = Curve3::Line(Line::through(a, b)?);
                uprights.push(body.add_edge(line, (0.0, a.distance(b)), from, to));
            }
        }
        for (i, side) in profile.sides.iter().enumerate() {
            let j = (i + 1) % n;
            let (Some(&(low, low_against)), Some(&(high, high_against)), Some(&p), Some(&q)) = (
                bottom.edges.get(i),
                top.edges.get(i),
                profile.corners.get(i),
                profile.corners.get(j),
            ) else {
                return Err(invalid("a profile side is missing"));
            };
            // Seen from outside, which is to the right of the outline:
            // along the bottom, up, back along the top, down.
            let loops = match (uprights.get(i), uprights.get(j)) {
                (Some(&here), Some(&there)) => vec![body.add_loop(vec![
                    coedge(low, low_against),
                    coedge(there, false),
                    coedge(high, !high_against),
                    coedge(here, true),
                ])],
                _ => vec![
                    body.add_loop(vec![coedge(low, low_against)]),
                    body.add_loop(vec![coedge(high, !high_against)]),
                ],
            };
            let (surface, reversed) = match *side {
                Side::Line => {
                    let start = frame.point(p.0, p.1, 0.0);
                    let along = frame.point(q.0, q.1, 0.0) - start;
                    let wall = frame_at(start, along.cross(frame.z()), along)?;
                    (Surface::Plane(Plane::new(wall)), false)
                }
                Side::Arc {
                    centre,
                    radius,
                    clockwise,
                } => {
                    let axis =
                        frame_at(frame.point(centre.0, centre.1, 0.0), frame.z(), frame.x())?;
                    // Going clockwise the outside is towards the axis.
                    (Surface::Cylinder(Cylinder::new(axis, radius)?), clockwise)
                }
            };
            faces.push(body.add_face(surface, reversed, loops));
        }
        floor_loops.push(
            body.add_loop(
                bottom
                    .edges
                    .iter()
                    .rev()
                    .map(|&(edge, against)| coedge(edge, !against))
                    .collect(),
            ),
        );
        lid_loops.push(
            body.add_loop(
                top.edges
                    .iter()
                    .map(|&(edge, against)| coedge(edge, against))
                    .collect(),
            ),
        );
    }
    let floor = frame_at(frame.origin(), -frame.z(), frame.x())?;
    let lid = frame_at(frame.point(0.0, 0.0, height), frame.z(), frame.x())?;
    faces.push(body.add_face(Surface::Plane(Plane::new(floor)), false, floor_loops));
    faces.push(body.add_face(Surface::Plane(Plane::new(lid)), false, lid_loops));
    body.add_shell(faces);
    Ok(body)
}

/// A plate `dx` × `dy` × `dz` with one corner at the frame's origin and a
/// round hole of `radius` through its middle, along z: planar faces with
/// a circular hole, and a cylindrical wall that faces inward.
pub fn plate_with_round_hole(
    frame: &Frame,
    dx: f64,
    dy: f64,
    dz: f64,
    radius: f64,
) -> Result<Reference, GeomError> {
    positive(&[dx, dy, dz, radius])?;
    if 2.0 * radius >= dx.min(dy) {
        return Err(invalid("the hole must fit in the plate"));
    }
    let centre = (dx / 2.0, dy / 2.0);
    let hole = Profile {
        corners: vec![(centre.0 + radius, centre.1)],
        sides: vec![Side::Arc {
            centre,
            radius,
            clockwise: true,
        }],
    };
    let body = extrude(frame, &[rectangle((0.0, dx), (0.0, dy), false), hole], dz)?;
    let disc = PI * radius * radius;
    Ok(Reference {
        name: "plate with round hole",
        body,
        volume: (dx * dy - disc) * dz,
        area: 2.0 * (dx * dy - disc) + 2.0 * (dx + dy) * dz + TAU * radius * dz,
        centroid: frame.point(dx / 2.0, dy / 2.0, dz / 2.0),
        counts: (10, 14, 7),
        genus: 1,
    })
}

/// A block `dx` × `dy` × `dz` with one corner at the frame's origin and a
/// rectangular hole `hx` × `hy` through its middle, along z: planar faces
/// with polygonal holes.
pub fn block_with_rectangular_hole(
    frame: &Frame,
    dx: f64,
    dy: f64,
    dz: f64,
    hx: f64,
    hy: f64,
) -> Result<Reference, GeomError> {
    positive(&[dx, dy, dz, hx, hy])?;
    if hx >= dx || hy >= dy {
        return Err(invalid("the hole must fit in the block"));
    }
    let hole = rectangle(
        ((dx - hx) / 2.0, (dx + hx) / 2.0),
        ((dy - hy) / 2.0, (dy + hy) / 2.0),
        true,
    );
    let body = extrude(frame, &[rectangle((0.0, dx), (0.0, dy), false), hole], dz)?;
    Ok(Reference {
        name: "block with rectangular hole",
        body,
        volume: (dx * dy - hx * hy) * dz,
        area: 2.0 * (dx * dy - hx * hy) + 2.0 * (dx + dy + hx + hy) * dz,
        centroid: frame.point(dx / 2.0, dy / 2.0, dz / 2.0),
        counts: (16, 24, 10),
        genus: 1,
    })
}

/// The half with y ≥ 0 of a cylinder of `radius` about the frame's z
/// axis, from its xy plane up to `height`: a wall bounded by two half
/// circles and two lines, D-shaped ends and a rectangular flat face.
pub fn half_cylinder(frame: &Frame, radius: f64, height: f64) -> Result<Reference, GeomError> {
    positive(&[radius, height])?;
    let profile = Profile {
        corners: vec![(radius, 0.0), (-radius, 0.0)],
        sides: vec![
            Side::Arc {
                centre: (0.0, 0.0),
                radius,
                clockwise: false,
            },
            Side::Line,
        ],
    };
    let body = extrude(frame, &[profile], height)?;
    Ok(Reference {
        name: "half cylinder",
        body,
        volume: PI * radius * radius * height / 2.0,
        area: PI * radius * radius + PI * radius * height + 2.0 * radius * height,
        // The centroid of a half disc is 4r / 3π from its straight side.
        centroid: frame.point(0.0, 4.0 * radius / (3.0 * PI), height / 2.0),
        counts: (4, 6, 4),
        genus: 0,
    })
}

/// The cap of height `height` cut off a sphere of `radius` about the
/// frame's origin by the plane z = `radius − height`: a sphere face
/// bounded by a circle that is not the equator (unless the two are
/// equal), closed by a disc.
pub fn spherical_cap(frame: &Frame, radius: f64, height: f64) -> Result<Reference, GeomError> {
    positive(&[radius, height])?;
    if height >= 2.0 * radius {
        return Err(invalid("the cap must be lower than the sphere"));
    }
    let level = radius - height;
    let rim = (height * (2.0 * radius - height)).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let disc_frame = frame_at(frame.point(0.0, 0.0, level), frame.z(), frame.x())?;
    let circle = full_circle(&mut body, disc_frame, rim)?;
    let dome_loop = body.add_loop(vec![coedge(circle, false)]);
    let dome = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius)?),
        false,
        vec![dome_loop],
    );
    let floor_loop = body.add_loop(vec![coedge(circle, true)]);
    let floor_frame = frame_at(frame.point(0.0, 0.0, level), -frame.z(), frame.x())?;
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    body.add_shell(vec![dome, floor]);
    Ok(Reference {
        name: "spherical cap",
        body,
        volume: PI * height * height * (3.0 * radius - height) / 3.0,
        area: TAU * radius * height + PI * rim * rim,
        // From the sphere's centre: 3 (2r − h)² / (4 (3r − h)).
        centroid: frame.point(
            0.0,
            0.0,
            3.0 * (2.0 * radius - height).powi(2) / (4.0 * (3.0 * radius - height)),
        ),
        counts: (1, 1, 2),
        genus: 0,
    })
}

/// A cone frustum standing on the frame's xy plane: radius `bottom`
/// there and `top` at `height`. A cone face between two circles.
pub fn cone_frustum(
    frame: &Frame,
    bottom: f64,
    top: f64,
    height: f64,
) -> Result<Reference, GeomError> {
    positive(&[bottom, top, height])?;
    let mut body = Body::new(BodyKind::Solid);
    let top_frame = frame_at(frame.point(0.0, 0.0, height), frame.z(), frame.x())?;
    let low = full_circle(&mut body, *frame, bottom)?;
    let high = full_circle(&mut body, top_frame, top)?;
    let surface = Surface::Cone(Cone::new(*frame, bottom, ((top - bottom) / height).atan())?);
    let wall_loops = vec![
        body.add_loop(vec![coedge(low, false)]),
        body.add_loop(vec![coedge(high, true)]),
    ];
    let wall = body.add_face(surface, false, wall_loops);
    let floor_loop = body.add_loop(vec![coedge(low, true)]);
    let floor_frame = frame_at(frame.origin(), -frame.z(), frame.x())?;
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![coedge(high, false)]);
    let lid = body.add_face(Surface::Plane(Plane::new(top_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    let squares = bottom * bottom + bottom * top + top * top;
    Ok(Reference {
        name: "cone frustum",
        body,
        volume: PI * height * squares / 3.0,
        area: PI * (bottom * bottom + top * top) + PI * (bottom + top) * height.hypot(bottom - top),
        centroid: frame.point(
            0.0,
            0.0,
            height * (bottom * bottom + 2.0 * bottom * top + 3.0 * top * top) / (4.0 * squares),
        ),
        counts: (2, 2, 3),
        genus: 0,
    })
}

/// A cylinder of `radius` about the frame's z axis, standing on its xy
/// plane and cut by an oblique plane: its height is `low` at x = −radius
/// and `high` at x = +radius. The cut is an ellipse edge and a planar
/// face bounded by it.
pub fn oblique_cylinder(
    frame: &Frame,
    radius: f64,
    low: f64,
    high: f64,
) -> Result<Reference, GeomError> {
    positive(&[radius, low, high])?;
    let mean = (low + high) / 2.0;
    let slope = (high - low) / (2.0 * radius);
    let mut body = Body::new(BodyKind::Solid);
    let bottom = full_circle(&mut body, *frame, radius)?;
    // The cut plane z = mean + slope x: its normal, and the ellipse's
    // major axis along the slope. With that axis the ellipse's parameter
    // is the cylinder's angle.
    let stretch = slope.hypot(1.0);
    let cut_frame = frame_at(
        frame.point(0.0, 0.0, mean),
        frame.vector(-slope, 0.0, 1.0),
        frame.vector(1.0, 0.0, slope),
    )?;
    let vertex = body.add_vertex(frame.point(radius, 0.0, high));
    let ellipse = Curve3::Ellipse(Ellipse::new(cut_frame, radius * stretch, radius)?);
    let cut = body.add_edge(ellipse, (0.0, TAU), vertex, vertex);
    let wall_loops = vec![
        body.add_loop(vec![coedge(bottom, false)]),
        body.add_loop(vec![coedge(cut, true)]),
    ];
    let wall = body.add_face(
        Surface::Cylinder(Cylinder::new(*frame, radius)?),
        false,
        wall_loops,
    );
    let floor_loop = body.add_loop(vec![coedge(bottom, true)]);
    let floor_frame = frame_at(frame.origin(), -frame.z(), frame.x())?;
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![coedge(cut, false)]);
    let lid = body.add_face(Surface::Plane(Plane::new(cut_frame)), false, vec![lid_loop]);
    body.add_shell(vec![wall, floor, lid]);
    let disc = PI * radius * radius;
    Ok(Reference {
        name: "oblique cylinder",
        body,
        volume: disc * mean,
        area: disc + TAU * radius * mean + disc * stretch,
        // Moments of the height field mean + slope x over the disc.
        centroid: frame.point(
            slope * radius * radius / (4.0 * mean),
            0.0,
            mean / 2.0 + slope * slope * radius * radius / (8.0 * mean),
        ),
        counts: (2, 2, 3),
        genus: 0,
    })
}

/// A block `dx` × `dy` standing on the frame's xy plane under a curved
/// roof: a bicubic Bézier patch (or, with `nurbs`, the same patch as a
/// NURBS surface) whose four boundary curves are cubic spline edges, on
/// top of four planar walls each bounded by three lines and a spline.
/// The roof is about `height` above the floor.
///
/// The roof's control points form a regular grid in x and y, so the
/// volume has a closed form: `dx × dy ×` the mean of the control heights
/// (every Bernstein polynomial has the same integral). The area and the
/// centroid have none: there the reference is `craft3d_topo`'s
/// `mass_properties` of this very body, a second, independent computation
/// (integrals over the exact geometry) rather than a formula.
pub fn domed_block(
    frame: &Frame,
    dx: f64,
    dy: f64,
    height: f64,
    nurbs: bool,
) -> Result<Reference, GeomError> {
    positive(&[dx, dy, height])?;
    let mut net = [[Vec3::ZERO; 4]; 4];
    let mut sum = 0.0;
    for (i, row) in net.iter_mut().enumerate() {
        for (j, point) in row.iter_mut().enumerate() {
            // Heights between 0.7 and 1.3 times `height`, without symmetry.
            let z = height * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) as f64 - 2.0));
            sum += z;
            *point = frame.point(dx * i as f64 / 3.0, dy * j as f64 / 3.0, z);
        }
    }
    let patch = BezierPatch::bicubic(net)?;
    let mut body = Body::new(BodyKind::Solid);
    let corner = |i: usize, j: usize| patch.control_point(i, j).ok_or(invalid("patch corner"));
    let ground = [(0.0, 0.0), (dx, 0.0), (dx, dy), (0.0, dy)];
    let low: Vec<VertexId> = ground
        .iter()
        .map(|&(x, y)| body.add_vertex(frame.point(x, y, 0.0)))
        .collect();
    let mut high = Vec::new();
    for (i, j) in [(0, 0), (3, 0), (3, 3), (0, 3)] {
        high.push(body.add_vertex(corner(i, j)?));
    }
    let cubic = |points: Vec<Vec3>| -> Result<Curve3, GeomError> {
        Ok(Curve3::Nurbs(NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            points,
        )?))
    };
    let along_u = |j: usize| -> Result<Curve3, GeomError> {
        cubic((0..4).map(|i| corner(i, j)).collect::<Result<_, _>>()?)
    };
    let along_v = |i: usize| -> Result<Curve3, GeomError> {
        cubic((0..4).map(|j| corner(i, j)).collect::<Result<_, _>>()?)
    };
    let (&[l0, l1, l2, l3], &[h0, h1, h2, h3]) = (low.as_slice(), high.as_slice()) else {
        return Err(invalid("a block has four corners"));
    };
    // Roof edges in the patch's own directions, and whether each runs
    // along the counter-clockwise outline of the roof.
    let roof = [
        (body.add_edge(along_u(0)?, (0.0, 1.0), h0, h1), true),
        (body.add_edge(along_v(3)?, (0.0, 1.0), h1, h2), true),
        (body.add_edge(along_u(3)?, (0.0, 1.0), h3, h2), false),
        (body.add_edge(along_v(0)?, (0.0, 1.0), h0, h3), false),
    ];
    let mut straight = |from: VertexId, to: VertexId| -> Result<EdgeId, GeomError> {
        let (Some(a), Some(b)) = (body.vertex(from), body.vertex(to)) else {
            return Err(invalid("a block corner is missing"));
        };
        let (a, b) = (a.point, b.point);
        let line = Curve3::Line(Line::through(a, b)?);
        Ok(body.add_edge(line, (0.0, a.distance(b)), from, to))
    };
    let floor_edges = [
        straight(l0, l1)?,
        straight(l1, l2)?,
        straight(l2, l3)?,
        straight(l3, l0)?,
    ];
    let uprights = [
        straight(l0, h0)?,
        straight(l1, h1)?,
        straight(l2, h2)?,
        straight(l3, h3)?,
    ];
    let mut faces = Vec::new();
    for k in 0..4 {
        let next = (k + 1) % 4;
        let (Some(&floor), Some(&(top, along)), Some(&here), Some(&there), Some(&p), Some(&q)) = (
            floor_edges.get(k),
            roof.get(k),
            uprights.get(k),
            uprights.get(next),
            ground.get(k),
            ground.get(next),
        ) else {
            return Err(invalid("a block has four walls"));
        };
        let loop_ = body.add_loop(vec![
            coedge(floor, false),
            coedge(there, false),
            coedge(top, along),
            coedge(here, true),
        ]);
        let start = frame.point(p.0, p.1, 0.0);
        let direction = frame.point(q.0, q.1, 0.0) - start;
        let wall = frame_at(start, direction.cross(frame.z()), direction)?;
        faces.push(body.add_face(Surface::Plane(Plane::new(wall)), false, vec![loop_]));
    }
    let floor_loop = body.add_loop(
        floor_edges
            .iter()
            .rev()
            .map(|&edge| coedge(edge, true))
            .collect(),
    );
    let floor_frame = frame_at(frame.origin(), -frame.z(), frame.x())?;
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    ));
    let roof_loop = body.add_loop(
        roof.iter()
            .map(|&(edge, along)| coedge(edge, !along))
            .collect(),
    );
    let surface = if nurbs {
        Surface::Nurbs(NurbsSurface::from_bezier(&patch)?)
    } else {
        Surface::Bezier(patch)
    };
    faces.push(body.add_face(surface, false, vec![roof_loop]));
    body.add_shell(faces);
    let exact = mass_properties(&body).map_err(|_| invalid("the domed block has no mass"))?;
    Ok(Reference {
        name: if nurbs {
            "domed block (nurbs roof)"
        } else {
            "domed block"
        },
        body,
        volume: dx * dy * sum / 16.0,
        area: exact.area,
        centroid: exact
            .centroid
            .ok_or(invalid("the domed block has no volume"))?,
        counts: (8, 12, 6),
        genus: 0,
    })
}

/// A pipe elbow: the part of a torus (`major`, `minor`) about the frame's
/// z axis between the angles 0 and `sweep`, closed by two discs. Its
/// curved face is bounded by two circles that wind around the *tube* of
/// the torus.
///
/// Volume and area follow from Pappus's theorems. Unlike the other
/// fixtures this one cannot be confirmed by `craft3d_topo`'s
/// `mass_properties`, which does not handle loops around the tube (it
/// returns a wrong area for this body); it is therefore not part of
/// [`all`].
pub fn pipe_elbow(
    frame: &Frame,
    major: f64,
    minor: f64,
    sweep: f64,
) -> Result<Reference, GeomError> {
    positive(&[major, minor, sweep])?;
    if sweep >= TAU {
        return Err(invalid("an elbow turns less than a full circle"));
    }
    let mut body = Body::new(BodyKind::Solid);
    // The tube's cross-section at angle `u`: a circle whose parameter is
    // the torus's v, about the direction of decreasing u.
    let section = |u: f64| -> Result<Frame, GeomError> {
        let outward = frame.vector(u.cos(), u.sin(), 0.0);
        let forward = frame.vector(-u.sin(), u.cos(), 0.0);
        frame_at(
            frame.point(major * u.cos(), major * u.sin(), 0.0),
            -forward,
            outward,
        )
    };
    let start_frame = section(0.0)?;
    let end_frame = section(sweep)?;
    let start = full_circle(&mut body, start_frame, minor)?;
    let end = full_circle(&mut body, end_frame, minor)?;
    // The face lies to the left of its loops: going up v, that is towards
    // lower u.
    let wall_loops = vec![
        body.add_loop(vec![coedge(start, true)]),
        body.add_loop(vec![coedge(end, false)]),
    ];
    let wall = body.add_face(
        Surface::Torus(Torus::new(*frame, major, minor)?),
        false,
        wall_loops,
    );
    let start_loop = body.add_loop(vec![coedge(start, false)]);
    let start_disc = body.add_face(
        Surface::Plane(Plane::new(start_frame)),
        false,
        vec![start_loop],
    );
    let end_loop = body.add_loop(vec![coedge(end, true)]);
    let end_disc_frame = frame_at(end_frame.origin(), -end_frame.z(), end_frame.x())?;
    let end_disc = body.add_face(
        Surface::Plane(Plane::new(end_disc_frame)),
        false,
        vec![end_loop],
    );
    body.add_shell(vec![wall, start_disc, end_disc]);
    let disc = PI * minor * minor;
    // The swept disc's second moment puts the centroid of each slice at
    // R + r² / 4R from the axis; over the sweep that averages to the
    // chord factor sin(θ/2) / (θ/2) along the bisector.
    let reach = (major + minor * minor / (4.0 * major)) * (sweep / 2.0).sin() / (sweep / 2.0);
    Ok(Reference {
        name: "pipe elbow",
        body,
        volume: disc * major * sweep,
        area: TAU * minor * major * sweep + 2.0 * disc,
        centroid: frame.point(
            reach * (sweep / 2.0).cos(),
            reach * (sweep / 2.0).sin(),
            0.0,
        ),
        counts: (2, 2, 3),
        genus: 0,
    })
}

/// The eighth of a sphere of `radius` about the frame's origin with x, y
/// and z all positive: a sphere face whose boundary passes *through* the
/// pole, where two of its three arcs meet, closed by three quarter discs.
///
/// Like the elbow it cannot be confirmed by `craft3d_topo`'s
/// `mass_properties` (which misses the part of the parameter boundary
/// that the pole stands for), so it is not part of [`all`]; its values
/// are an eighth of the sphere's and three quarter discs.
pub fn sphere_octant(frame: &Frame, radius: f64) -> Result<Reference, GeomError> {
    positive(&[radius])?;
    let mut body = Body::new(BodyKind::Solid);
    let (x, y, z) = (frame.x(), frame.y(), frame.z());
    let o = frame.origin();
    let centre = body.add_vertex(o);
    let a = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let b = body.add_vertex(frame.point(0.0, radius, 0.0));
    let pole = body.add_vertex(frame.point(0.0, 0.0, radius));
    let quarter = (0.0, PI / 2.0);
    let arc = |axis: Vec3, from: Vec3| -> Result<Curve3, GeomError> {
        Ok(Curve3::Circle(Circle::new(
            frame_at(o, axis, from)?,
            radius,
        )?))
    };
    let equator = body.add_edge(arc(z, x)?, quarter, a, b);
    let b_up = body.add_edge(arc(x, y)?, quarter, b, pole);
    let a_up = body.add_edge(arc(-y, x)?, quarter, a, pole);
    let spoke = |to: Vec3| -> Result<Curve3, GeomError> { Ok(Curve3::Line(Line::new(o, to)?)) };
    let to_a = body.add_edge(spoke(x)?, (0.0, radius), centre, a);
    let to_b = body.add_edge(spoke(y)?, (0.0, radius), centre, b);
    let to_pole = body.add_edge(spoke(z)?, (0.0, radius), centre, pole);
    let dome_loop = body.add_loop(vec![
        coedge(equator, false),
        coedge(b_up, false),
        coedge(a_up, true),
    ]);
    let dome = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius)?),
        false,
        vec![dome_loop],
    );
    let mut flat = |normal: Vec3, along: Vec3, coedges: Vec<Coedge>| -> Result<_, GeomError> {
        let loop_ = body.add_loop(coedges);
        let plane = Surface::Plane(Plane::new(frame_at(o, normal, along)?));
        Ok(body.add_face(plane, false, vec![loop_]))
    };
    let floor = flat(
        -z,
        x,
        vec![
            coedge(to_b, false),
            coedge(equator, true),
            coedge(to_a, true),
        ],
    )?;
    let back = flat(
        -x,
        y,
        vec![
            coedge(b_up, true),
            coedge(to_b, true),
            coedge(to_pole, false),
        ],
    )?;
    let side = flat(
        -y,
        x,
        vec![
            coedge(a_up, false),
            coedge(to_pole, true),
            coedge(to_a, false),
        ],
    )?;
    body.add_shell(vec![dome, floor, back, side]);
    Ok(Reference {
        name: "sphere octant",
        body,
        volume: PI * radius.powi(3) / 6.0,
        area: PI * radius * radius / 2.0 + 3.0 * PI * radius * radius / 4.0,
        centroid: frame.point(3.0 * radius / 8.0, 3.0 * radius / 8.0, 3.0 * radius / 8.0),
        counts: (4, 6, 4),
        genus: 0,
    })
}

/// One of each trimmed reference solid whose values `mass_properties`
/// can confirm, with dimensions of the order of `scale`.
pub fn all(frame: &Frame, scale: f64) -> Result<Vec<Reference>, GeomError> {
    Ok(vec![
        plate_with_round_hole(frame, 5.0 * scale, 4.0 * scale, 1.5 * scale, 1.2 * scale)?,
        block_with_rectangular_hole(
            frame,
            5.0 * scale,
            4.0 * scale,
            2.0 * scale,
            2.0 * scale,
            1.0 * scale,
        )?,
        half_cylinder(frame, 1.5 * scale, 3.0 * scale)?,
        spherical_cap(frame, 2.5 * scale, 1.0 * scale)?,
        cone_frustum(frame, 2.0 * scale, 0.8 * scale, 2.5 * scale)?,
        oblique_cylinder(frame, 1.5 * scale, 1.0 * scale, 3.0 * scale)?,
        domed_block(frame, 3.0 * scale, 2.0 * scale, 1.5 * scale, false)?,
        domed_block(frame, 3.0 * scale, 2.0 * scale, 1.5 * scale, true)?,
    ])
}
