//! The section across a straight edge one of whose faces is a cylinder.
//!
//! A straight edge on a cylinder is one of its rulings, parallel to its
//! axis, and a plane through that line holds the axis' direction. So the
//! two faces look the same in every plane across the edge, and so does
//! the blend: in that plane each face is a line (a plane) or a circle (a
//! cylinder). A fillet is the circle of the radius tangent to both: its
//! centre is where the two curves, moved by the radius towards the ball,
//! cross. A chamfer is the chord between the points the distances away
//! from the edge, measured along each curve. Swept along the edge, the
//! fillet face is a cylinder and the chamfer face a plane, and the tool is
//! an extrusion, as between two planes. (The cut edges of a halved rod or
//! tube; the sharp leading edge of a blade made of two arcs.)

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{BlendError, Size, refused};
use crate::chamfer::ChamferSize;
use craft3d_geom::Surface;
use craft3d_math::Vec3;
use craft3d_topo::{EdgeId, FaceId};
use std::f64::consts::PI;

/// A trace may run at most this far round its circle: the tool's section
/// is drawn with the short arc.
const MOST_TURN: f64 = PI * (1.0 - 1e-6);

/// A face in the plane across the edge. Offsets are from the edge's
/// point in that plane.
#[derive(Clone, Copy, Debug)]
pub(super) enum Trace {
    /// A plane: the line from the edge along `into`.
    Line { into: Vec3 },
    /// A cylinder: the circle about `centre` through the edge's point,
    /// which it leaves along `into`.
    Circle {
        centre: Vec3,
        radius: f64,
        into: Vec3,
    },
}

impl Trace {
    /// The unit vector from the circle's centre to the edge.
    fn out(centre: Vec3, radius: f64) -> Vec3 {
        centre * (-1.0 / radius)
    }

    /// The point `s` along the trace from the edge, measured along it.
    pub(super) fn at(&self, s: f64) -> Vec3 {
        match *self {
            Trace::Line { into } => into * s,
            Trace::Circle {
                centre,
                radius,
                into,
            } => {
                let phi = s / radius;
                centre + (Trace::out(centre, radius) * phi.cos() + into * phi.sin()) * radius
            }
        }
    }

    /// The unit direction of the trace at `s`, away from the edge.
    fn direction(&self, s: f64) -> Vec3 {
        match *self {
            Trace::Line { into } => into,
            Trace::Circle {
                centre,
                radius,
                into,
            } => {
                let phi = s / radius;
                into * phi.cos() - Trace::out(centre, radius) * phi.sin()
            }
        }
    }

    /// How far along the trace the point of it nearest `offset` lies:
    /// negative behind the edge; round a circle within half a turn either
    /// way.
    pub(super) fn coordinate(&self, offset: Vec3) -> f64 {
        match *self {
            Trace::Line { into } => offset.dot(into),
            Trace::Circle {
                centre,
                radius,
                into,
            } => {
                let d = offset - centre;
                radius * d.dot(into).atan2(d.dot(Trace::out(centre, radius)))
            }
        }
    }

    /// The circle's centre and radius, for a circle.
    pub(super) fn circle(&self) -> Option<(Vec3, f64)> {
        match *self {
            Trace::Line { .. } => None,
            Trace::Circle { centre, radius, .. } => Some((centre, radius)),
        }
    }
}

/// The trace of a face along a straight edge from `start` along `along`,
/// left along `into`: `None` for a surface that does not look the same in
/// every plane across the edge.
pub(super) fn trace(
    surface: &Surface,
    start: Vec3,
    along: Vec3,
    into: Vec3,
    tol: f64,
) -> Option<Trace> {
    match surface {
        Surface::Plane(_) => Some(Trace::Line { into }),
        Surface::Cylinder(cylinder) => {
            let axis = cylinder.frame().z();
            if axis.cross(along).length() > 1e-9 {
                return None;
            }
            let offset = cylinder.frame().origin() - start;
            let centre = offset - along * offset.dot(along);
            let radius = cylinder.radius();
            let fits = radius.is_finite()
                && radius > 0.0
                && (centre.length() - radius).abs() <= 2.0 * tol.max(1e-12 * radius);
            fits.then_some(Trace::Circle {
                centre,
                radius,
                into,
            })
        }
        Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Bezier(_)
        | Surface::Nurbs(_) => None,
    }
}

/// The blend in the plane across the edge.
#[derive(Clone, Copy, Debug)]
pub(super) struct Section {
    pub traces: [Trace; 2],
    /// Where the blend leaves each face: offsets from the edge.
    pub feet: [Vec3; 2],
    /// How far along each face that is from the edge.
    pub distances: [f64; 2],
    /// A fillet's ball: the offset of its centre.
    pub centre: Option<Vec3>,
}

/// A curve in the plane across the edge on which the ball's centre lies:
/// the points `height` along the unit `normal`, or the points `radius`
/// from `centre`.
#[derive(Clone, Copy, Debug)]
enum Locus {
    Line { normal: [f64; 2], height: f64 },
    Circle { centre: [f64; 2], radius: f64 },
}

fn crossings(a: Locus, b: Locus) -> Vec<[f64; 2]> {
    match (a, b) {
        (
            Locus::Line {
                normal: n1,
                height: h1,
            },
            Locus::Line {
                normal: n2,
                height: h2,
            },
        ) => {
            let det = n1[0] * n2[1] - n1[1] * n2[0];
            if det.abs() <= 1e-12 {
                return Vec::new();
            }
            vec![[
                (h1 * n2[1] - h2 * n1[1]) / det,
                (n1[0] * h2 - n2[0] * h1) / det,
            ]]
        }
        (Locus::Line { normal, height }, Locus::Circle { centre, radius })
        | (Locus::Circle { centre, radius }, Locus::Line { normal, height }) => {
            let foot = [normal[0] * height, normal[1] * height];
            let run = [-normal[1], normal[0]];
            let q = [foot[0] - centre[0], foot[1] - centre[1]];
            let half_b = run[0] * q[0] + run[1] * q[1];
            let c = q[0] * q[0] + q[1] * q[1] - radius * radius;
            let disc = half_b * half_b - c;
            if !(disc >= 0.0) {
                return Vec::new();
            }
            let root = disc.sqrt();
            [-half_b - root, -half_b + root]
                .into_iter()
                .map(|t| [foot[0] + run[0] * t, foot[1] + run[1] * t])
                .collect()
        }
        (
            Locus::Circle {
                centre: c1,
                radius: r1,
            },
            Locus::Circle {
                centre: c2,
                radius: r2,
            },
        ) => {
            let d = [c2[0] - c1[0], c2[1] - c1[1]];
            let dist = d[0].hypot(d[1]);
            if !(dist > 1e-12 * r1.max(r2)) {
                return Vec::new();
            }
            let a = (r1 * r1 - r2 * r2 + dist * dist) / (2.0 * dist);
            let h2 = r1 * r1 - a * a;
            if !(h2 >= 0.0) {
                return Vec::new();
            }
            let h = h2.sqrt();
            let (ex, ey) = (d[0] / dist, d[1] / dist);
            let base = [c1[0] + ex * a, c1[1] + ey * a];
            vec![
                [base[0] - ey * h, base[1] + ex * h],
                [base[0] + ey * h, base[1] - ex * h],
            ]
        }
    }
}

/// Works out the section of the blend of `edge` between the faces whose
/// traces are `traces` and whose outward normals at the edge are
/// `normals`. `estimate` is where the ball's centre would be if both faces
/// were their tangent planes at the edge, which picks the right one of
/// the crossings.
#[allow(clippy::too_many_arguments)]
pub(super) fn section(
    edge: EdgeId,
    faces: [FaceId; 2],
    traces: [Trace; 2],
    normals: [Vec3; 2],
    along: Vec3,
    convex: bool,
    size: Size,
    estimate: Vec3,
    tol: f64,
) -> Result<Section, BlendError> {
    let [t1, t2] = traces;
    let too_large = |side: usize, size: f64, largest: f64| BlendError::TooLarge {
        edge,
        face: faces.get(side).copied().unwrap_or(faces[0]),
        size,
        largest: largest.max(0.0),
    };
    // How far round its circle a trace may go.
    let most = |trace: &Trace| trace.circle().map_or(f64::INFINITY, |(_, r)| MOST_TURN * r);
    match size {
        Size::Radius(radius) => {
            let e1 = match t1 {
                Trace::Line { into } | Trace::Circle { into, .. } => into,
            };
            let e2 = along.cross(e1);
            let flat = |v: Vec3| [v.dot(e1), v.dot(e2)];
            let away = if convex { -radius } else { radius };
            let mut loci = Vec::with_capacity(2);
            for (side, (trace, normal)) in traces.iter().zip(normals).enumerate() {
                loci.push(match *trace {
                    Trace::Line { .. } => {
                        let n = flat(normal);
                        Locus::Line {
                            normal: n,
                            height: away,
                        }
                    }
                    Trace::Circle {
                        centre,
                        radius: rim,
                        ..
                    } => {
                        // Outward away from the centre (a rod) or towards
                        // it (a hole).
                        let bulge = normal.dot(Trace::out(centre, rim)).signum();
                        let moved = rim + bulge * away;
                        if !(moved > 2.0 * tol) {
                            return Err(too_large(side, radius, rim));
                        }
                        Locus::Circle {
                            centre: flat(centre),
                            radius: moved,
                        }
                    }
                });
            }
            let [la, lb] = loci.as_slice() else {
                return Err(super::lost());
            };
            let guess = flat(estimate);
            let best = crossings(*la, *lb).into_iter().min_by(|p, q| {
                let dp = (p[0] - guess[0]).hypot(p[1] - guess[1]);
                let dq = (q[0] - guess[0]).hypot(q[1] - guess[1]);
                dp.total_cmp(&dq)
            });
            let Some([x, y]) = best else {
                // The two curves moved by the radius do not meet: the ball
                // does not fit between the faces.
                return Err(too_large(0, radius, 0.0));
            };
            let centre = e1 * x + e2 * y;
            let mut feet = [Vec3::new(0.0, 0.0, 0.0); 2];
            let mut distances = [0.0; 2];
            for (side, trace) in traces.iter().enumerate() {
                let foot = match *trace {
                    Trace::Line { into } => into * centre.dot(into),
                    Trace::Circle {
                        centre: middle,
                        radius: rim,
                        ..
                    } => {
                        let Some(dir) = (centre - middle).normalized() else {
                            return Err(too_large(side, radius, 0.0));
                        };
                        middle + dir * rim
                    }
                };
                let s = trace.coordinate(foot);
                if !(s > 16.0 * tol && s < most(trace)) {
                    return Err(too_large(side, radius, 0.0));
                }
                if let (Some(f), Some(d)) = (feet.get_mut(side), distances.get_mut(side)) {
                    *f = foot;
                    *d = s;
                }
            }
            Ok(Section {
                traces,
                feet,
                distances,
                centre: Some(centre),
            })
        }
        Size::Chamfer(ChamferSize::Distances { first, second }) => {
            let distances = [first, second];
            let mut feet = [Vec3::new(0.0, 0.0, 0.0); 2];
            for (side, (trace, &s)) in traces.iter().zip(&distances).enumerate() {
                if !(s < most(trace)) {
                    return Err(too_large(side, s, most(trace)));
                }
                if let Some(f) = feet.get_mut(side) {
                    *f = trace.at(s);
                }
            }
            Ok(Section {
                traces,
                feet,
                distances,
                centre: None,
            })
        }
        Size::Chamfer(ChamferSize::DistanceAngle { distance, angle }) => {
            if !(distance < most(&t1)) {
                return Err(too_large(0, distance, most(&t1)));
            }
            if !(angle.is_finite() && angle > 0.0 && angle < PI) {
                return Err(refused(edge, super::Reason::Angle));
            }
            let first = t1.at(distance);
            let back = -t1.direction(distance);
            let into2 = match t2 {
                Trace::Line { into } | Trace::Circle { into, .. } => into,
            };
            let into1 = match t1 {
                Trace::Line { into } | Trace::Circle { into, .. } => into,
            };
            // The chamfer line leaves the first foot at the angle to the
            // first face, turned towards the second face.
            let turn = if along.dot(into1.cross(into2)) >= 0.0 {
                -angle
            } else {
                angle
            };
            let dir = back * turn.cos() + along.cross(back) * turn.sin();
            let hit = match t2 {
                Trace::Line { into } => {
                    let den = along.dot(dir.cross(into));
                    if den.abs() <= 1e-12 {
                        None
                    } else {
                        let lambda = -along.dot(first.cross(into)) / den;
                        (lambda > 0.0).then_some(first + dir * lambda)
                    }
                }
                Trace::Circle { centre, radius, .. } => {
                    let q = first - centre;
                    let half_b = dir.dot(q);
                    let c = q.dot(q) - radius * radius;
                    let disc = half_b * half_b - c;
                    if disc >= 0.0 {
                        let root = disc.sqrt();
                        [-half_b - root, -half_b + root]
                            .into_iter()
                            .filter(|l| *l > 0.0)
                            .map(|l| first + dir * l)
                            .find(|p| t2.coordinate(*p) > 0.0)
                    } else {
                        None
                    }
                }
            };
            let Some(second) = hit else {
                return Err(refused(edge, super::Reason::Angle));
            };
            let s2 = t2.coordinate(second);
            if !(s2 > 0.0 && s2 < most(&t2)) {
                return Err(refused(edge, super::Reason::Angle));
            }
            Ok(Section {
                traces,
                feet: [first, second],
                distances: [distance, s2],
                centre: None,
            })
        }
    }
}
