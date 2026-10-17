//! Reference solids with their closed-form mass properties (oracle 1 in
//! docs/testing.md), placed by a frame so tests can tilt and scale them.
//! The primitives come from `craft3d-kernel`; the hemisphere is built by
//! hand here.

use craft3d_geom::{Circle, Curve3, GeomError, Plane, Sphere, Surface};
use craft3d_kernel::{KernelError, primitives};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId};
use std::f64::consts::{PI, TAU};

/// A body and what is known about it exactly.
#[derive(Clone, Debug)]
pub struct Reference {
    pub name: &'static str,
    pub body: Body,
    pub volume: f64,
    pub area: f64,
    pub centroid: Vec3,
    /// `(vertices, edges, faces)`.
    pub counts: (usize, usize, usize),
    pub genus: u32,
}

fn invalid(message: &'static str) -> GeomError {
    GeomError::InvalidParameter(message)
}

/// The fixtures use valid sizes; a kernel error here is a geometry error.
fn geometry(error: KernelError) -> GeomError {
    match error {
        KernelError::Geometry(e) => e,
        KernelError::InvalidSize { .. } => invalid("sizes must be positive and finite"),
        // Profile and extrusion errors: the primitives used here never
        // produce them.
        _ => invalid("a primitive failed in a way only sketches can"),
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
    frame.reversed_at(origin).ok_or(invalid("degenerate frame"))
}

/// A full circle of `radius` about the z axis of `frame`, with its one
/// vertex on the x axis.
fn full_circle(body: &mut Body, frame: Frame, radius: f64) -> Result<EdgeId, GeomError> {
    let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
    let circle = Curve3::Circle(Circle::new(frame, radius)?);
    Ok(body.add_edge(circle, (0.0, TAU), vertex, vertex))
}

/// A rectangular block with one corner at the frame's origin and edges
/// `dx`, `dy`, `dz` along its axes.
pub fn block(frame: &Frame, dx: f64, dy: f64, dz: f64) -> Result<Reference, GeomError> {
    let body = primitives::block(frame, dx, dy, dz).map_err(geometry)?;
    Ok(Reference {
        name: "block",
        body,
        volume: dx * dy * dz,
        area: 2.0 * (dx * dy + dy * dz + dz * dx),
        centroid: frame.point(dx / 2.0, dy / 2.0, dz / 2.0),
        counts: (8, 12, 6),
        genus: 0,
    })
}

/// A cylinder standing on the frame's xy plane, about its z axis.
pub fn cylinder(frame: &Frame, radius: f64, height: f64) -> Result<Reference, GeomError> {
    let body = primitives::cylinder(frame, radius, height).map_err(geometry)?;
    Ok(Reference {
        name: "cylinder",
        body,
        volume: PI * radius * radius * height,
        area: TAU * radius * (radius + height),
        centroid: frame.point(0.0, 0.0, height / 2.0),
        counts: (2, 2, 3),
        genus: 0,
    })
}

/// A cone standing on the frame's xy plane with its apex at `height` on
/// the z axis.
pub fn cone(frame: &Frame, radius: f64, height: f64) -> Result<Reference, GeomError> {
    let body = primitives::cone(frame, radius, height).map_err(geometry)?;
    Ok(Reference {
        name: "cone",
        body,
        volume: PI * radius * radius * height / 3.0,
        area: PI * radius * (radius + radius.hypot(height)),
        centroid: frame.point(0.0, 0.0, height / 4.0),
        counts: (1, 1, 2),
        genus: 0,
    })
}

/// The cone a straight cubic spline sweeps from the axis, turned about
/// the frame's y axis: its apex at the frame's origin (a pole of the
/// first order of its spline surface, where every ruling is straight),
/// its base a disc of `radius` at `height` up the y axis.
pub fn spline_cone(frame: &Frame, radius: f64, height: f64) -> Result<Reference, GeomError> {
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    let tolerance = craft3d_math::Tolerance::new(1e-7 * radius.max(height), 1e-9)
        .ok_or(invalid("sizes must be positive and finite"))?;
    let at = |s: f64| [radius * s, height * s];
    let curves = vec![
        SketchCurve {
            id: 3,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: vec![at(0.0), at(1.0 / 3.0), at(2.0 / 3.0), at(1.0)],
            }),
        },
        SketchCurve {
            id: 5,
            curve: Curve2::Line {
                start: at(1.0),
                end: [0.0, height],
            },
        },
        SketchCurve {
            id: 7,
            curve: Curve2::Line {
                start: [0.0, height],
                end: at(0.0),
            },
        },
    ];
    let profiles = detect_profiles(&curves, &tolerance).map_err(geometry)?;
    let body = craft3d_kernel::revolve::revolve(
        &profiles,
        frame,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        TAU,
        &tolerance,
    )
    .map_err(geometry)?
    .body;
    // (Its counts as the revolve makes them: no closed form to hold.)
    let counts = (body.vertices.len(), body.edges.len(), body.faces.len());
    Ok(Reference {
        name: "spline cone",
        body,
        volume: PI * radius * radius * height / 3.0,
        area: PI * radius * (radius + radius.hypot(height)),
        centroid: frame.point(0.0, 0.75 * height, 0.0),
        counts,
        genus: 0,
    })
}

/// A sphere about the frame's origin: one face, no edges.
pub fn sphere(frame: &Frame, radius: f64) -> Result<Reference, GeomError> {
    let body = primitives::sphere(frame, radius).map_err(geometry)?;
    Ok(Reference {
        name: "sphere",
        body,
        volume: 4.0 / 3.0 * PI * radius.powi(3),
        area: 4.0 * PI * radius * radius,
        centroid: frame.origin(),
        counts: (0, 0, 1),
        genus: 0,
    })
}

/// The half of a sphere above the frame's xy plane, closed by a disc.
pub fn hemisphere(frame: &Frame, radius: f64) -> Result<Reference, GeomError> {
    let mut body = Body::new(BodyKind::Solid);
    let equator = full_circle(&mut body, *frame, radius)?;
    let dome_loop = body.add_loop(vec![forward(equator)]);
    let dome = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius)?),
        false,
        vec![dome_loop],
    );
    let floor_loop = body.add_loop(vec![backward(equator)]);
    let floor = body.add_face(
        Surface::Plane(Plane::new(flipped(frame, frame.origin())?)),
        false,
        vec![floor_loop],
    );
    body.add_shell(vec![dome, floor]);
    Ok(Reference {
        name: "hemisphere",
        body,
        volume: 2.0 / 3.0 * PI * radius.powi(3),
        area: 3.0 * PI * radius * radius,
        centroid: frame.point(0.0, 0.0, 3.0 * radius / 8.0),
        counts: (1, 1, 2),
        genus: 0,
    })
}

/// A ring torus about the frame's z axis: one face, no edges, one handle.
pub fn torus(frame: &Frame, major: f64, minor: f64) -> Result<Reference, GeomError> {
    let body = primitives::torus(frame, major, minor).map_err(geometry)?;
    Ok(Reference {
        name: "torus",
        body,
        volume: 2.0 * PI * PI * major * minor * minor,
        area: 4.0 * PI * PI * major * minor,
        centroid: frame.origin(),
        counts: (0, 0, 1),
        genus: 1,
    })
}

/// One of each reference solid, with dimensions of the order of `scale`.
pub fn all(frame: &Frame, scale: f64) -> Result<Vec<Reference>, GeomError> {
    Ok(vec![
        block(frame, 4.0 * scale, 2.0 * scale, 1.0 * scale)?,
        cylinder(frame, 1.5 * scale, 4.0 * scale)?,
        cone(frame, 2.0 * scale, 3.0 * scale)?,
        sphere(frame, 2.5 * scale)?,
        hemisphere(frame, 2.0 * scale)?,
        torus(frame, 3.0 * scale, 1.0 * scale)?,
    ])
}
