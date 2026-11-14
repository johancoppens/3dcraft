//! Shared by the hunts of the face operations (`hunt_*.rs`): seeded bodies
//! of many kinds (blocks, cylinders, cones, balls, rings, their unions
//! and cuts, spline walls and grooves, lofts, filleted and chamfered
//! blocks), at several scales and in several frames, and oracles that do
//! not ask how an operation built its result: the volume and area by the
//! integrator against the volume of the mesh, the same operation on the
//! body moved far away and turned, and with an unrelated small body
//! added far away, and points classified by rays.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::boolean_common::Rng;
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_tess::{SelfCheckParams, body_self_intersections};
use craft3d_topo::{Body, EdgeId, FaceId, check, edge_uses, mass_bound, merge};
use std::f64::consts::{PI, TAU};

/// The kinds of body the hunts operate on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Block,
    Cylinder,
    Frustum,
    Ball,
    Ring,
    /// The block with a cylinder lying in its top, united or taken out.
    Lying,
    /// The block with a cylinder standing through its top.
    Standing,
    /// The block with a cone standing through its top.
    Cone,
    /// The block with a ball half sunk in its top.
    BallIn,
    /// The block with a ring (torus) lying in its top.
    RingIn,
    /// The block with its top shaped by a spline hump, or such a groove.
    Hump,
    /// The block with a cylinder lying at a slant through its top.
    Tilted,
    /// The block with a cone lying along it in its top.
    LyingCone,
    /// The block with an edge filleted.
    Filleted,
    /// The block with an edge chamfered.
    Chamfered,
    /// A prism on a profile with a spline side.
    SplineWall,
    /// A loft from a rectangle to a circle.
    Loft,
    /// A spline profile turned about an axis: a vase.
    Vase,
    /// A smooth loft through three rectangles, turned and moved.
    Twisted,
}

pub const KINDS: [Kind; 19] = [
    Kind::Block,
    Kind::Cylinder,
    Kind::Frustum,
    Kind::Ball,
    Kind::Ring,
    Kind::Lying,
    Kind::Standing,
    Kind::Cone,
    Kind::BallIn,
    Kind::RingIn,
    Kind::Hump,
    Kind::Tilted,
    Kind::LyingCone,
    Kind::Filleted,
    Kind::Chamfered,
    Kind::SplineWall,
    Kind::Loft,
    Kind::Vase,
    Kind::Twisted,
];

/// The scales the hunts run at (rule 3: 1e-3 to 1e6 model units).
pub const SCALES: [f64; 4] = [1e-3, 1.0, 1.0, 1e3];

pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// A body made for a case, and what it is.
pub struct Made {
    pub body: Body,
    pub what: String,
    pub kind: Kind,
    pub union: bool,
    pub frame: Frame,
    pub k: f64,
    /// The sizes drawn, in model units at scale 1: a block's three, a
    /// cylinder's radius and height, a ball's radius, a ring's two radii,
    /// a frustum's radius, height and cut; a filleted or chamfered block's
    /// size, number of edges and first edge; empty for the others.
    pub dims: Vec<f64>,
}

/// A frame at scale `k`: the world, or turned and moved some way off.
pub fn frame(rng: &mut Rng, k: f64) -> Frame {
    if rng.below(3) == 0 {
        return Frame::WORLD;
    }
    let origin = Vec3::new(
        rng.between(-200.0, 200.0) * k,
        rng.between(-200.0, 200.0) * k,
        rng.between(-200.0, 200.0) * k,
    );
    let z = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let x = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    Frame::new(origin, z, x).unwrap_or(Frame::WORLD)
}

/// A rigid motion that takes a body of size `size` some 125 times its
/// size away and turns it.
pub fn far_motion(rng: &mut Rng, size: f64) -> Transform {
    let axis = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(0.2, 1.0),
    );
    let turn =
        Transform::rotation_about(Vec3::new(0.0, 0.0, 0.0), axis, rng.between(0.3, 2.8)).unwrap();
    let away = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let away = away.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0)) * (125.0 * size);
    turn.then(&Transform::translation(away))
}

/// The block from (0, 0, 0) to (40, 30, 20) times `k` in `frame`.
fn block(frame: &Frame, k: f64) -> Body {
    primitives::block(frame, 40.0 * k, 30.0 * k, 20.0 * k).unwrap()
}

/// The block with its top shaped across y by a cubic spline from (a, 20)
/// to (b, 20): a ridge (`union`) or a groove (mirrored into the block).
fn ridge(
    frame: &Frame,
    k: f64,
    tol: &Tolerance,
    (a, b): (f64, f64),
    knots: Vec<f64>,
    control: Vec<[f64; 2]>,
    union: bool,
) -> Option<Body> {
    let line = |id: u64, start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [start[0] * k, start[1] * k],
            end: [end[0] * k, end[1] * k],
        },
    };
    let control = control
        .into_iter()
        .map(|[y, z]| if union { [y, z] } else { [y, 40.0 - z] })
        .map(|[y, z]| [y * k, z * k])
        .collect();
    let curves = vec![
        line(1, [0.0, 0.0], [30.0, 0.0]),
        line(2, [30.0, 0.0], [30.0, 20.0]),
        line(3, [30.0, 20.0], [a, 20.0]),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots,
                control,
            }),
        },
        line(5, [b, 20.0], [0.0, 20.0]),
        line(6, [0.0, 20.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tol).ok()?;
    let along = Frame::new(frame.origin(), frame.x(), frame.y())?;
    let body = extrude(&profiles, &along, 0.0, 40.0 * k, tol).ok()?.body;
    check(&body, tol).is_empty().then_some(body)
}

/// The edge of `body` whose middle is nearest `p`.
pub fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

/// A body of `kind` at scale `k` in `frame`, its sizes drawn from `rng`;
/// `None` where the kernel does not build it (not an operation's to
/// judge).
pub fn make(rng: &mut Rng, kind: Kind, k: f64, tol: &Tolerance) -> Option<Made> {
    let frame = frame(rng, k);
    let union = rng.below(2) == 0;
    let p = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
    let at = |origin: Vec3, z: Vec3, x: Vec3| Frame::new(origin, z, x).unwrap();
    let combine = |other: Body| -> Option<Body> {
        let op = if union {
            BooleanOp::Union
        } else {
            BooleanOp::Subtract
        };
        let made = boolean(&block(&frame, k), &other, op, tol).ok()?.body;
        check(&made, tol).is_empty().then_some(made)
    };
    let mut dims = Vec::new();
    let (body, what) = match kind {
        Kind::Block => {
            let (a, b, c) = (
                rng.between(10.0, 50.0),
                rng.between(10.0, 50.0),
                rng.between(5.0, 40.0),
            );
            dims = vec![a, b, c];
            (
                primitives::block(&frame, a * k, b * k, c * k).ok()?,
                format!("block {a:.3} x {b:.3} x {c:.3}"),
            )
        }
        Kind::Cylinder => {
            let (r, h) = (rng.between(3.0, 20.0), rng.between(5.0, 40.0));
            dims = vec![r, h];
            (
                primitives::cylinder(&frame, r * k, h * k).ok()?,
                format!("cylinder r {r:.3} h {h:.3}"),
            )
        }
        Kind::Frustum => {
            // A cone cut level below its apex.
            let (r, h) = (rng.between(5.0, 20.0), rng.between(10.0, 40.0));
            let cut = rng.between(0.3, 0.8) * h;
            let cone = primitives::cone(&frame, r * k, h * k).ok()?;
            let lid = primitives::block(
                &at(p(-2.0 * r, -2.0 * r, cut), frame.z(), frame.x()),
                4.0 * r * k,
                4.0 * r * k,
                h * k,
            )
            .ok()?;
            let made = boolean(&cone, &lid, BooleanOp::Subtract, tol).ok()?.body;
            dims = vec![r, h, cut];
            (
                check(&made, tol).is_empty().then_some(made)?,
                format!("frustum r {r:.3} h {h:.3} cut at {cut:.3}"),
            )
        }
        Kind::Ball => {
            let r = rng.between(3.0, 20.0);
            dims = vec![r];
            (
                primitives::sphere(&frame, r * k).ok()?,
                format!("ball r {r:.3}"),
            )
        }
        Kind::Ring => {
            let (major, minor) = (rng.between(6.0, 20.0), rng.between(1.0, 5.0));
            dims = vec![major, minor];
            (
                primitives::torus(&frame, major * k, minor * k).ok()?,
                format!("ring {major:.3} / {minor:.3}"),
            )
        }
        Kind::Lying => {
            let r = rng.between(3.0, 9.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let other =
                primitives::cylinder(&at(p(-1.0, y, z), frame.x(), frame.y()), r * k, 42.0 * k)
                    .ok()?;
            (
                combine(other)?,
                format!("block with a lying cylinder r {r:.3} at y {y:.3} z {z:.3}"),
            )
        }
        Kind::Standing => {
            let r = rng.between(2.0, 7.0);
            let (x, y, z) = (
                rng.between(12.0, 28.0),
                rng.between(10.0, 20.0),
                rng.between(5.0, 15.0),
            );
            let other =
                primitives::cylinder(&at(p(x, y, z), frame.z(), frame.x()), r * k, 20.0 * k)
                    .ok()?;
            (
                combine(other)?,
                format!("block with a standing cylinder r {r:.3} at {x:.3} {y:.3} from z {z:.3}"),
            )
        }
        Kind::Cone => {
            let r = rng.between(4.0, 9.0);
            let (x, y, z) = (
                rng.between(12.0, 28.0),
                rng.between(10.0, 20.0),
                rng.between(8.0, 16.0),
            );
            let h = rng.between(8.0, 20.0);
            let other =
                primitives::cone(&at(p(x, y, z), frame.z(), frame.x()), r * k, h * k).ok()?;
            (
                combine(other)?,
                format!("block with a cone r {r:.3} h {h:.3} at {x:.3} {y:.3} from z {z:.3}"),
            )
        }
        Kind::BallIn => {
            let r = rng.between(3.0, 9.0);
            let (x, y) = (rng.between(12.0, 28.0), rng.between(10.0, 20.0));
            let z = 20.0 + rng.between(-0.8, 0.8) * r;
            let other = primitives::sphere(&at(p(x, y, z), frame.z(), frame.x()), r * k).ok()?;
            (
                combine(other)?,
                format!("block with a ball r {r:.3} at {x:.3} {y:.3} {z:.3}"),
            )
        }
        Kind::RingIn => {
            let (major, minor) = (rng.between(5.0, 9.0), rng.between(1.5, 3.5));
            let (x, y) = (rng.between(15.0, 25.0), rng.between(12.0, 18.0));
            let z = 20.0 + rng.between(-0.7, 0.7) * minor;
            let other =
                primitives::torus(&at(p(x, y, z), frame.z(), frame.x()), major * k, minor * k)
                    .ok()?;
            (
                combine(other)?,
                format!("block with a ring {major:.3} / {minor:.3} at {x:.3} {y:.3} {z:.3}"),
            )
        }
        Kind::Hump => {
            let (a, b) = (rng.between(18.0, 24.0), rng.between(6.0, 12.0));
            let h = rng.between(4.0, 12.0);
            let lean = rng.between(-2.0, 6.0);
            let control = vec![
                [a, 20.0],
                [a + lean, 20.0 + h],
                [b - lean, 20.0 + h],
                [b, 20.0],
            ];
            (
                ridge(
                    &frame,
                    k,
                    tol,
                    (a, b),
                    vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control,
                    union,
                )?,
                format!("block with a spline hump {a:.3} to {b:.3} h {h:.3} lean {lean:.3}"),
            )
        }
        Kind::Tilted => {
            let r = rng.between(3.0, 8.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let direction = frame.x()
                + frame.y() * rng.between(-0.3, 0.3)
                + frame.z() * rng.between(-0.15, 0.15);
            let axis = at(p(20.0, y, z) - direction * (30.0 * k), direction, frame.z());
            let other = primitives::cylinder(&axis, r * k, 60.0 * k * direction.length()).ok()?;
            (
                combine(other)?,
                format!("block with a slanted cylinder r {r:.3} at y {y:.3} z {z:.3}"),
            )
        }
        Kind::LyingCone => {
            let r = rng.between(5.0, 9.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let h = rng.between(70.0, 200.0);
            let other =
                primitives::cone(&at(p(-1.0, y, z), frame.x(), frame.y()), r * k, h * k).ok()?;
            (
                combine(other)?,
                format!("block with a lying cone r {r:.3} h {h:.3} at y {y:.3} z {z:.3}"),
            )
        }
        Kind::Filleted | Kind::Chamfered => {
            let b = block(&frame, k);
            // One of the block's twelve edges, by its middle.
            let middles = [
                (20.0, 0.0, 0.0),
                (20.0, 30.0, 0.0),
                (20.0, 0.0, 20.0),
                (20.0, 30.0, 20.0),
                (0.0, 15.0, 0.0),
                (40.0, 15.0, 0.0),
                (0.0, 15.0, 20.0),
                (40.0, 15.0, 20.0),
                (0.0, 0.0, 10.0),
                (40.0, 0.0, 10.0),
                (0.0, 30.0, 10.0),
                (40.0, 30.0, 10.0),
            ];
            let count = 1 + rng.below(3) as usize;
            let first = rng.below(12) as usize;
            let picked: Vec<EdgeId> = (0..count)
                .map(|i| {
                    let (x, y, z) = middles[(first + 4 * i) % 12];
                    edge_near(&b, p(x, y, z))
                })
                .collect();
            let size = rng.between(0.5, 6.0);
            dims = vec![size, count as f64, first as f64];
            if kind == Kind::Filleted {
                let edges: Vec<FilletEdge> = picked
                    .iter()
                    .map(|e| FilletEdge {
                        edge: *e,
                        radius: size * k,
                    })
                    .collect();
                (
                    fillet(&b, &edges, tol).ok()?.body,
                    format!("block with {count} edges from {first} filleted {size:.3}"),
                )
            } else {
                let uses = edge_uses(&b);
                let edges: Vec<ChamferEdge> = picked
                    .iter()
                    .map(|e| ChamferEdge {
                        edge: *e,
                        first: uses[e.0 as usize][0].face,
                        size: ChamferSize::Distances {
                            first: size * k,
                            second: size * k * 0.7,
                        },
                    })
                    .collect();
                (
                    chamfer(&b, &edges, tol).ok()?.body,
                    format!("block with {count} edges from {first} chamfered {size:.3}"),
                )
            }
        }
        Kind::SplineWall => {
            // A profile with three straight sides and a wavy spline one,
            // extruded.
            let (w, d) = (rng.between(20.0, 40.0), rng.between(15.0, 30.0));
            let bulge = [
                rng.between(-4.0, 4.0),
                rng.between(-4.0, 4.0),
                rng.between(-4.0, 4.0),
            ];
            let h = rng.between(8.0, 30.0);
            let line = |id: u64, s: [f64; 2], e: [f64; 2]| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: [s[0] * k, s[1] * k],
                    end: [e[0] * k, e[1] * k],
                },
            };
            let control: Vec<[f64; 2]> = vec![
                [w, d],
                [0.75 * w, d + bulge[0]],
                [0.5 * w, d + bulge[1]],
                [0.25 * w, d + bulge[2]],
                [0.0, d],
            ]
            .into_iter()
            .map(|[x, y]| [x * k, y * k])
            .collect();
            let curves = vec![
                line(1, [0.0, 0.0], [w, 0.0]),
                line(2, [w, 0.0], [w, d]),
                SketchCurve {
                    id: 3,
                    curve: Curve2::Spline(Spline2 {
                        degree: 3,
                        knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                        control,
                    }),
                },
                line(4, [0.0, d], [0.0, 0.0]),
            ];
            let profiles = detect_profiles(&curves, tol).ok()?;
            let body = extrude(&profiles, &frame, 0.0, h * k, tol).ok()?.body;
            (
                check(&body, tol).is_empty().then_some(body)?,
                format!(
                    "spline wall {w:.3} x {d:.3} h {h:.3} bulges {:.3} {:.3} {:.3}",
                    bulge[0], bulge[1], bulge[2]
                ),
            )
        }
        Kind::Loft => {
            let (a, b) = (rng.between(10.0, 30.0), rng.between(10.0, 30.0));
            let r = rng.between(5.0, 15.0);
            let h = rng.between(10.0, 40.0);
            let line = |id: u64, s: [f64; 2], e: [f64; 2]| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: [s[0] * k, s[1] * k],
                    end: [e[0] * k, e[1] * k],
                },
            };
            let rect = vec![
                line(1, [-a / 2.0, -b / 2.0], [a / 2.0, -b / 2.0]),
                line(2, [a / 2.0, -b / 2.0], [a / 2.0, b / 2.0]),
                line(3, [a / 2.0, b / 2.0], [-a / 2.0, b / 2.0]),
                line(4, [-a / 2.0, b / 2.0], [-a / 2.0, -b / 2.0]),
            ];
            let circle = vec![SketchCurve {
                id: 10,
                curve: Curve2::Circle {
                    center: [0.0, 0.0],
                    radius: r * k,
                },
            }];
            let section = |curves: &[SketchCurve], plane: Frame| -> Option<Section> {
                let profiles = detect_profiles(curves, tol).ok()?;
                Some(Section {
                    plane,
                    outline: profiles.first()?.outer.clone(),
                    start: None,
                })
            };
            let top = frame.at(p(0.0, 0.0, h))?;
            let lofted = loft(
                &[section(&rect, frame)?, section(&circle, top)?],
                &Options::default(),
                tol,
            )
            .ok()?;
            (
                lofted.body,
                format!("loft from {a:.3} x {b:.3} to a circle r {r:.3} over {h:.3}"),
            )
        }
        Kind::Vase => {
            let (r0, r1, h) = (
                rng.between(5.0, 15.0),
                rng.between(4.0, 15.0),
                rng.between(10.0, 40.0),
            );
            let bulge = [rng.between(-3.0, 6.0), rng.between(-3.0, 6.0)];
            let line = |id: u64, s: [f64; 2], e: [f64; 2]| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: [s[0] * k, s[1] * k],
                    end: [e[0] * k, e[1] * k],
                },
            };
            let mid = 0.5 * (r0 + r1);
            let control: Vec<[f64; 2]> = vec![
                [r0, 0.0],
                [r0 + bulge[0], h / 3.0],
                [mid + bulge[1], 2.0 * h / 3.0],
                [r1, h],
            ]
            .into_iter()
            .map(|[x, y]| [x * k, y * k])
            .collect();
            let curves = vec![
                line(1, [0.0, 0.0], [r0, 0.0]),
                SketchCurve {
                    id: 2,
                    curve: Curve2::Spline(Spline2 {
                        degree: 3,
                        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                        control,
                    }),
                },
                line(3, [r1, h], [0.0, h]),
                line(4, [0.0, h], [0.0, 0.0]),
            ];
            let profiles = detect_profiles(&curves, tol).ok()?;
            let plane = Frame::new(frame.origin(), -frame.y(), frame.x())?;
            let body = craft3d_kernel::revolve::revolve(
                &profiles,
                &plane,
                [0.0, 0.0],
                [0.0, 1.0],
                0.0,
                std::f64::consts::TAU,
                tol,
            )
            .ok()?
            .body;
            (
                body,
                format!(
                    "vase r {r0:.3} to {r1:.3} over {h:.3}, bulges {:.3} {:.3}",
                    bulge[0], bulge[1]
                ),
            )
        }
        Kind::Twisted => {
            let (a, b) = (rng.between(10.0, 30.0), rng.between(10.0, 30.0));
            let h = rng.between(15.0, 40.0);
            let turn = rng.between(-25.0, 25.0).to_radians();
            let shrink = rng.between(0.6, 1.2);
            let shift = [rng.between(-4.0, 4.0), rng.between(-4.0, 4.0)];
            let rect = |first: u64, scale: f64, angle: f64| -> Vec<SketchCurve> {
                let (s, c) = (angle.psin(), angle.pcos());
                let corners: Vec<[f64; 2]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                    .iter()
                    .map(|[x, y]| {
                        let (x, y) = (x * 0.5 * a * scale, y * 0.5 * b * scale);
                        [(c * x - s * y) * k, (s * x + c * y) * k]
                    })
                    .collect();
                (0..4)
                    .map(|i| SketchCurve {
                        id: first + i as u64,
                        curve: Curve2::Line {
                            start: corners[i],
                            end: corners[(i + 1) % 4],
                        },
                    })
                    .collect()
            };
            let section = |curves: &[SketchCurve], plane: Frame| -> Option<Section> {
                let profiles = detect_profiles(curves, tol).ok()?;
                Some(Section {
                    plane,
                    outline: profiles.first()?.outer.clone(),
                    start: None,
                })
            };
            let middle = frame.at(p(0.5 * shift[0], 0.5 * shift[1], 0.5 * h))?;
            let top = frame.at(p(shift[0], shift[1], h))?;
            let lofted = loft(
                &[
                    section(&rect(1, 1.0, 0.0), frame)?,
                    section(&rect(11, 0.5 * (1.0 + shrink), 0.5 * turn), middle)?,
                    section(&rect(21, shrink, turn), top)?,
                ],
                &Options::default(),
                tol,
            )
            .ok()?;
            (
                lofted.body,
                format!(
                    "smooth loft of rectangles {a:.3} x {b:.3} over {h:.3}, turned {:.3}, scaled {shrink:.3}, moved {:.3} {:.3}",
                    turn.to_degrees(),
                    shift[0],
                    shift[1]
                ),
            )
        }
    };
    if !check(&body, tol).is_empty() {
        return None;
    }
    let what = match kind {
        Kind::Lying
        | Kind::Standing
        | Kind::Cone
        | Kind::BallIn
        | Kind::RingIn
        | Kind::Hump
        | Kind::Tilted
        | Kind::LyingCone => format!(
            "{what}, {}, scale {k}",
            if union { "united" } else { "taken out" }
        ),
        _ => format!("{what}, scale {k}"),
    };
    Some(Made {
        body,
        what,
        kind,
        union,
        frame,
        k,
        dims,
    })
}

/// A body's volume, area and the integrator's own bound on its volume.
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    pub volume: f64,
    pub area: f64,
    pub bound: f64,
}

pub fn measured(body: &Body) -> Option<Measured> {
    let (m, bound) = mass_bound(body).ok()?;
    (m.volume.is_finite() && m.area.is_finite()).then_some(Measured {
        volume: m.volume,
        area: m.area,
        bound: bound.volume,
    })
}

/// The corners of the box round the vertices (and the sample points of
/// the edges) of `body`.
pub fn bounds(body: &Body) -> (Vec3, Vec3) {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    let mut add = |p: Vec3| {
        lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    };
    for v in &body.vertices {
        add(v.point);
    }
    for e in &body.edges {
        for i in 0..=8 {
            add(e
                .curve
                .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(i) / 8.0));
        }
    }
    if let Some((min, max)) = body.bbox().ok().and_then(|b| b.corners()) {
        add(min);
        add(max);
    }
    (lo, hi)
}

pub fn size_of(body: &Body) -> f64 {
    let (lo, hi) = bounds(body);
    (hi - lo).length()
}

/// The known allowance of the measure for edges that lie off their faces
/// (the ribbon between the two faces' feet of an edge, counted as a cone
/// from the middle the moments are taken about; docs/design/face-operations.md
/// › *An offset that measured smaller*, Open): per edge, at 32 places, how
/// far it lies off each of its faces, times the length there and half the
/// distance from the middle of the body's box.
pub fn ribbon(body: &Body) -> f64 {
    let (lo, hi) = bounds(body);
    let middle = (lo + hi) * 0.5;
    let uses = edge_uses(body);
    let mut sum = 0.0;
    for (index, edge) in body.edges.iter().enumerate() {
        let Some(users) = uses.get(index) else {
            continue;
        };
        let steps = 32;
        let mut last = edge.curve.eval(edge.range.0);
        for i in 1..=steps {
            let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(i) / f64::from(steps);
            let q = edge.curve.eval(t);
            let dl = q.distance(last);
            last = q;
            let mut off = 0.0;
            for u in users {
                if let Some(face) = body.face(u.face)
                    && let Ok((s, v)) = face.surface.project(q)
                {
                    off += face.surface.eval(s, v).distance(q);
                }
            }
            sum += 0.5 * off * dl * q.distance(middle);
        }
    }
    sum
}

/// The volume of the mesh of `body` at the chordal tolerance `chord`,
/// welded and closed; an error says why there is none.
pub fn mesh_volume(body: &Body, chord: f64) -> Result<f64, String> {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .map_err(|e| format!("not meshed: {e}"))?;
    let welded = mesh.weld().map_err(|e| format!("not welded: {e:?}"))?;
    if !welded.is_closed() {
        return Err("the mesh is not closed".into());
    }
    Ok(welded.volume())
}

/// The mesh of `body` at the chordal tolerance `chord`.
pub fn mesh(body: &Body, chord: f64) -> Option<Mesh> {
    tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .ok()
}

/// The point of the triangle `a b c` nearest `p` (Ericson, Real-Time
/// Collision Detection, 5.1.5), and so its distance.
pub fn triangle_distance(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return p.distance(a);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return p.distance(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return p.distance(a + ab * v);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return p.distance(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return p.distance(a + ac * w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return p.distance(b + (c - b) * w);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    p.distance(a + ab * v + ac * w)
}

/// The distance from `p` to the triangles of `mesh` of the faces for
/// which `which` holds.
pub fn distance_to(mesh: &Mesh, which: &dyn Fn(u32) -> bool, p: Vec3) -> f64 {
    let mut best = f64::INFINITY;
    for (t, face) in mesh.face_ids.iter().enumerate() {
        if !which(*face) {
            continue;
        }
        let Some([a, b, c]) = mesh.corners(t) else {
            continue;
        };
        best = best.min(triangle_distance(p, a, b, c));
    }
    best
}

/// Points on the triangles of the faces of `mesh` for which `which`
/// holds, moved off them at random by up to `spread`; `count` of them.
pub fn points_near(
    rng: &mut Rng,
    mesh: &Mesh,
    which: &dyn Fn(u32) -> bool,
    spread: f64,
    count: usize,
) -> Vec<Vec3> {
    let picked: Vec<usize> = (0..mesh.triangles.len())
        .filter(|t| which(mesh.face_ids[*t]))
        .collect();
    if picked.is_empty() {
        return Vec::new();
    }
    (0..count)
        .map(|_| {
            let t = picked[rng.below(picked.len() as u64) as usize];
            let [a, b, c] = mesh.corners(t).unwrap();
            let (mut u, mut v) = (rng.unit(), rng.unit());
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let on = a + (b - a) * u + (c - a) * v;
            on + Vec3::new(
                rng.between(-1.0, 1.0),
                rng.between(-1.0, 1.0),
                rng.between(-1.0, 1.0),
            ) * spread
        })
        .collect()
}

/// Points anywhere in the box round `body` (widened by a quarter).
pub fn points_in_box(rng: &mut Rng, body: &Body, count: usize) -> Vec<Vec3> {
    let (lo, hi) = bounds(body);
    let margin = (hi - lo) * 0.25;
    let (lo, hi) = (lo - margin, hi + margin);
    (0..count)
        .map(|_| {
            Vec3::new(
                rng.between(lo.x, hi.x),
                rng.between(lo.y, hi.y),
                rng.between(lo.z, hi.z),
            )
        })
        .collect()
}

/// Points classified against `body`, or `None` where the rays cannot
/// tell.
pub fn classify(body: &Body, points: &[Vec3], tol: &Tolerance) -> Option<Vec<Place>> {
    classify_points(body, points, tol).ok()
}

/// `body` with a cube of side `k` placed `125` times its size away (on the
/// far side of the origin from it): an unrelated body that moves the
/// middle of the set the measure is taken about. Its faces, edges and
/// vertices keep their numbers; the cube's come after.
pub fn with_far_cube(body: &Body, k: f64) -> Option<(Body, f64)> {
    let (lo, hi) = bounds(body);
    let middle = (lo + hi) * 0.5;
    let size = (hi - lo).length();
    let dir = middle.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0));
    let at = middle - dir * (125.0 * size);
    let cube = primitives::block(&Frame::WORLD.at(at)?, k, k, k).ok()?;
    let (merged, _) = merge(&[body, &cube]).ok()?;
    Some((merged, k * k * k))
}

/// How far two measures of what should be the same solid may differ: the
/// integrator's bounds of both, the known ribbon of both, what two bodies
/// within the tolerance of each other differ by (half the tolerance over
/// the area: a fitted face is held to a quarter of it), and rounding.
pub fn allowance(a: &Measured, b: &Measured, ribbons: f64, tol: &Tolerance) -> f64 {
    a.bound + b.bound + ribbons + 0.5 * tol.linear * a.area.max(b.area) + 1e-10 * a.volume.abs()
}

/// What happened to a case, as a letter: `S` no body to operate on, `N`
/// nothing to pick, `B` built and held, `W` built and seen wrong, `I` an
/// invalid body returned, `M` a body that passes the checker but whose
/// faces cannot be meshed (the regeneration refuses it), `X` built, seen
/// wrong and passing through itself (the regeneration refuses it), `R`
/// refused, `P` a panic.
pub struct Tally {
    pub outcomes: String,
    pub wrong: Vec<String>,
    /// Per case, a short note of what the oracles could not look at.
    pub notes: Vec<String>,
}

impl Tally {
    pub fn new() -> Self {
        Tally {
            outcomes: String::new(),
            wrong: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn count(&self, c: char) -> usize {
        self.outcomes.chars().filter(|o| *o == c).count()
    }

    /// Prints the tally and holds it to `known`: the cases seen wrong
    /// that are known, by number, each with what is wrong.
    /// Prints the tally and holds it to `known` (the cases seen wrong
    /// that are known, by number) and to `pinned`, the outcomes recorded
    /// when the sweep was written (empty: not pinned): a case built right
    /// then is built right still, one set up then is set up still.
    pub fn hold_pinned(
        &self,
        name: &str,
        start: usize,
        known: &[usize],
        pinned: &str,
        least_built: usize,
    ) {
        let mut changed = Vec::new();
        for (i, (then, now)) in pinned.chars().zip(self.outcomes.chars()).enumerate() {
            let fine = match then {
                'B' => now == 'B',
                'S' | 'N' => now == then,
                _ => now != 'S' && now != 'N',
            };
            if !fine {
                changed.push(format!("case {}: {then} when pinned, {now} now", start + i));
            }
            if then != now {
                println!("{name}: case {}: {then} when pinned, {now} now", start + i);
            }
        }
        self.hold(name, start, known, least_built);
        assert!(changed.is_empty(), "{name}: {changed:#?}");
    }

    pub fn hold(&self, name: &str, start: usize, known: &[usize], least_built: usize) {
        println!(
            "{name}: built right {}, refused {}, wrong {}, invalid {}, unmeshable {}, through itself {}, panic {}, no body {}, nothing to pick {}",
            self.count('B'),
            self.count('R'),
            self.count('W'),
            self.count('I'),
            self.count('M'),
            self.count('X'),
            self.count('P'),
            self.count('S'),
            self.count('N'),
        );
        println!("{name}: outcomes from {start}: {}", self.outcomes);
        for line in &self.wrong {
            println!("  {line}");
        }
        for line in &self.notes {
            println!("  note: {line}");
        }
        let mut unknown = Vec::new();
        for (i, c) in self.outcomes.chars().enumerate() {
            let n = start + i;
            // A body that passes through itself (`X`) is refused by the
            // regeneration's self-check, which every face operation runs:
            // counted, not a finding.
            let bad = matches!(c, 'W' | 'I' | 'M' | 'P');
            if bad && !known.contains(&n) {
                unknown.push(format!("case {n}: {c}"));
            }
            if !bad && known.contains(&n) {
                unknown.push(format!(
                    "case {n}: known wrong, now {c}: take it off the list"
                ));
            }
        }
        assert!(unknown.is_empty(), "{name}: {unknown:#?}");
        assert!(
            self.count('B') >= least_built,
            "{name}: too few built to say much"
        );
    }
}

/// Runs `f`, a panic caught and returned as its message.
pub fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "a panic".into())
    })
}

/// The seed of case `n` of a sweep seeded `seed`: what one case draws does
/// not depend on how another came out.
pub fn case_seed(seed: u64, n: usize) -> u64 {
    seed ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The least sine of the angle between `face` and its neighbours across
/// its edges, at five places along each (1 where it has no edges).
pub fn least_sine(body: &Body, face: FaceId) -> f64 {
    let uses = edge_uses(body);
    let mut least: f64 = 1.0;
    let normal = |f: FaceId, p: Vec3| -> Option<Vec3> {
        let found = body.face(f)?;
        let (u, v) = found.surface.project(p).ok()?;
        let n = found.surface.normal(u, v)?;
        Some(if found.reversed { -n } else { n })
    };
    for (index, users) in uses.iter().enumerate() {
        let [a, b] = users.as_slice() else { continue };
        if a.face != face && b.face != face {
            continue;
        }
        let other = if a.face == face { b.face } else { a.face };
        let edge = &body.edges[index];
        for i in 1..6 {
            let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(i) / 6.0;
            let q = edge.curve.eval(t);
            if let (Some(n1), Some(n2)) = (normal(face, q), normal(other, q)) {
                least = least.min(n1.cross(n2).length());
            }
        }
    }
    least
}

/// The faces next to `face` across its edges.
pub fn neighbours(body: &Body, face: FaceId) -> Vec<FaceId> {
    let mut found = Vec::new();
    for users in edge_uses(body) {
        if users.iter().any(|u| u.face == face) {
            for u in &users {
                if u.face != face && !found.contains(&u.face) {
                    found.push(u.face);
                }
            }
        }
    }
    found
}

/// The outward normal of `face` at the point of it nearest `p`.
pub fn normal_at(body: &Body, face: FaceId, p: Vec3) -> Option<Vec3> {
    let f = body.face(face)?;
    let (u, v) = f.surface.project(p).ok()?;
    let n = f.surface.normal(u, v)?;
    Some(if f.reversed { -n } else { n })
}

/// The points of `mesh` of face `face`, in a short list (for normals).
pub fn face_points(mesh: &Mesh, face: u32) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (t, f) in mesh.face_ids.iter().enumerate() {
        if *f == face
            && let Some([a, b, c]) = mesh.corners(t)
        {
            out.push((a + b + c) * (1.0 / 3.0));
        }
    }
    out
}

/// The area and the centroid of the triangles of `face` in `mesh`.
pub fn face_area_centroid(mesh: &Mesh, face: u32) -> (f64, Vec3) {
    let mut area = 0.0;
    let mut moment = Vec3::new(0.0, 0.0, 0.0);
    for (t, f) in mesh.face_ids.iter().enumerate() {
        if *f != face {
            continue;
        }
        let Some([a, b, c]) = mesh.corners(t) else {
            continue;
        };
        let w = 0.5 * (b - a).cross(c - a).length();
        area += w;
        moment += (a + b + c) * (w / 3.0);
    }
    (area, moment * (1.0 / area.max(f64::MIN_POSITIVE)))
}

/// Writes `x` with seven significant digits.
pub fn g(x: f64) -> String {
    format!("{x:.7e}")
}

pub fn ppow(x: f64, n: i32) -> f64 {
    x.ppowi(n)
}

/// What the oracles found of one result.
#[derive(Default)]
pub struct Found {
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub invalid: bool,
    /// Passes the checker, but its faces cannot be meshed at any chord:
    /// the tessellator finds a face whose loops bound no region. The
    /// regeneration's self-check meshes every result and refuses it.
    pub unmeshable: bool,
}

pub fn identity() -> Transform {
    Transform::translation(Vec3::new(0.0, 0.0, 0.0))
}

/// Whether a result passes through itself (the tessellator's self-check,
/// which regeneration runs on the faces an operation changed).
pub fn crosses_itself(body: &Body, tol: &Tolerance) -> bool {
    body_self_intersections(body, &SelfCheckParams::new(tol.linear)).is_ok_and(|c| !c.is_clean())
}

/// The checks every result gets: valid, measurable, its mesh closed and of
/// the volume measured. The measure, where there is one.
pub fn sound(after: &Body, tol: &Tolerance, found: &mut Found) -> Option<Measured> {
    let defects = check(after, tol);
    if !defects.is_empty() {
        found.invalid = true;
        found
            .problems
            .push(format!("not valid: {:?}", defects.first()));
        return None;
    }
    let Some(m) = measured(after) else {
        found.problems.push("cannot be measured".into());
        return None;
    };
    if m.volume <= 0.0 {
        found
            .problems
            .push(format!("a volume not above zero: {}", g(m.volume)));
        return None;
    }
    let size = size_of(after);
    let chord = 1e-3 * size;
    match mesh_volume(after, chord) {
        Ok(v) => {
            let bound = 2.0 * chord * m.area + m.bound;
            if (v - m.volume).abs() > bound {
                found.problems.push(format!(
                    "the mesh's volume {} against the measured {} (bound {})",
                    g(v),
                    g(m.volume),
                    g(bound)
                ));
            }
        }
        Err(e) => {
            // Whether it is the chord: coarser and finer.
            let others: Vec<String> = [1e-2, 1e-4]
                .iter()
                .map(|f| match mesh_volume(after, f * size) {
                    Ok(v) => format!("at {f}: {} for {}", g(v), g(m.volume)),
                    Err(e) => format!("at {f}: {e}"),
                })
                .collect();
            if others.iter().all(|o| o.contains("not meshed")) {
                found.unmeshable = true;
                found
                    .problems
                    .push(format!("passes the checker but cannot be meshed: {e}"));
            } else {
                found.notes.push(format!("mesh: {e}; {others:?}"));
            }
        }
    }
    Some(m)
}

/// What an operation does to the far cube of [`frames`], and its side.
#[derive(Clone, Copy, Debug)]
pub enum Cube {
    /// It stays as it is (offset, move, draft, delete face).
    Kept(f64),
    /// It is made hollow with walls of the thickness (shell).
    Shelled(f64, f64),
    /// It is not in the result (thicken, whose layer is a body of its
    /// own).
    Dropped(f64),
}

impl Cube {
    fn side(self) -> f64 {
        match self {
            Cube::Kept(s) | Cube::Shelled(s, _) | Cube::Dropped(s) => s,
        }
    }

    /// The cube's volume and area in the result.
    fn after(self) -> (f64, f64) {
        match self {
            Cube::Kept(s) => (s * s * s, 6.0 * s * s),
            Cube::Shelled(s, t) => {
                let i = s - 2.0 * t;
                (s * s * s - i * i * i, 6.0 * (s * s + i * i))
            }
            Cube::Dropped(_) => (0.0, 0.0),
        }
    }
}

/// The same operation on `body` moved far away and turned, and on `body`
/// with a cube added far away: the same volume and area as `m` (the
/// result `made` in place).
#[allow(clippy::too_many_arguments)]
pub fn frames(
    rng: &mut Rng,
    body: &Body,
    made: &Body,
    m: &Measured,
    run: &dyn Fn(&Body, &Transform) -> Result<Body, KernelError>,
    cube: Cube,
    tol: &Tolerance,
    found: &mut Found,
) {
    let size = size_of(body);
    let motion = far_motion(rng, size);
    let ribbon_made = ribbon(made);
    if let Ok(moved) = body.transformed(&motion) {
        match guarded(|| run(&moved, &motion)) {
            Err(p) => found
                .problems
                .push(format!("a panic in the far frame: {p}")),
            Ok(Err(e)) => found.notes.push(format!("refused in the far frame: {e}")),
            Ok(Ok(other)) => {
                if !check(&other, tol).is_empty() {
                    found.invalid = true;
                    found.problems.push("not valid in the far frame".into());
                } else if let Some(mo) = measured(&other) {
                    let allowed = allowance(m, &mo, ribbon_made + ribbon(&other), tol);
                    if (mo.volume - m.volume).abs() > allowed {
                        found.problems.push(format!(
                            "in the far frame the volume is {} for {} (allowed {})",
                            g(mo.volume),
                            g(m.volume),
                            g(allowed)
                        ));
                    }
                    if (mo.area - m.area).abs() > 1e-7 * m.area + 4.0 * tol.linear * size {
                        found.problems.push(format!(
                            "in the far frame the area is {} for {}",
                            g(mo.area),
                            g(m.area)
                        ));
                    }
                } else {
                    found
                        .problems
                        .push("cannot be measured in the far frame".into());
                }
            }
        }
    }
    let Some((merged, _)) = with_far_cube(body, cube.side()) else {
        return;
    };
    let (cube_volume, cube_area) = cube.after();
    match guarded(|| run(&merged, &identity())) {
        Err(p) => found
            .problems
            .push(format!("a panic beside a far cube: {p}")),
        Ok(Err(e)) => found.notes.push(format!("refused beside a far cube: {e}")),
        Ok(Ok(other)) => {
            if !check(&other, tol).is_empty() {
                found.invalid = true;
                found.problems.push("not valid beside a far cube".into());
            } else if let Some(mo) = measured(&other) {
                let alone = Measured {
                    volume: mo.volume - cube_volume,
                    area: mo.area - cube_area,
                    bound: mo.bound,
                };
                let ribbons = ribbon(&other);
                let allowed = allowance(m, &alone, ribbon_made + ribbons, tol);
                let strict = allowance(m, &alone, 0.0, tol);
                let off = (alone.volume - m.volume).abs();
                if off > allowed {
                    found.problems.push(format!(
                        "beside a far cube the volume is {} for {} (allowed {}, of it the ribbon {})",
                        g(alone.volume),
                        g(m.volume),
                        g(allowed),
                        g(ribbons)
                    ));
                } else if off > strict {
                    found.notes.push(format!(
                        "beside a far cube {} off, within the known ribbon {}",
                        g(off),
                        g(ribbons)
                    ));
                }
            }
        }
    }
}

/// Points near `face` and in the box, classified against `before` and
/// `after`: with `adds` (an outward offset) none may leave the body, with
/// `!adds` none may enter it; none farther than `reach` from the face may
/// change. `None` for either: not held to that.
#[allow(clippy::too_many_arguments)]
pub fn points(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    faces: &[FaceId],
    adds: Option<bool>,
    reach: f64,
    tol: &Tolerance,
    found: &mut Found,
) {
    let size = size_of(before);
    let Some(mesh) = mesh(before, 2e-3 * size) else {
        found
            .notes
            .push("the body before could not be meshed".into());
        return;
    };
    let which = |f: u32| faces.iter().any(|x| x.0 == f);
    let mut pts = points_near(rng, &mesh, &which, 1.5 * reach, 120);
    pts.extend(points_in_box(rng, before, 60));
    let (Some(was), Some(is)) = (classify(before, &pts, tol), classify(after, &pts, tol)) else {
        found.notes.push("points could not be classified".into());
        return;
    };
    for ((p, a), b) in pts.iter().zip(&was).zip(&is) {
        let taken = *a == Place::Inside && *b == Place::Outside;
        let added = *a == Place::Outside && *b == Place::Inside;
        if !(taken || added) {
            continue;
        }
        let away = distance_to(&mesh, &which, *p);
        if adds == Some(true) && taken {
            found.problems.push(format!(
                "a point {} from the face taken from the body by an outward move",
                g(away)
            ));
            return;
        }
        if adds == Some(false) && added {
            found.problems.push(format!(
                "a point {} from the face added to the body by an inward move",
                g(away)
            ));
            return;
        }
        // The mesh lies within its chord of the face.
        if away > reach + 2e-3 * size + 20.0 * tol.linear {
            found.problems.push(format!(
                "a point {} from the face {}, past the reach {}",
                g(away),
                if added { "added" } else { "taken" },
                g(reach)
            ));
            return;
        }
    }
}

/// Whether every neighbour of the flat face `face` stands square to it: a
/// plane square to it or a cylinder along its normal.
pub fn prism_like(body: &Body, face: FaceId) -> Option<Vec3> {
    let found = body.face(face)?;
    let Surface::Plane(plane) = &found.surface else {
        return None;
    };
    let n = plane.frame().z();
    for other in neighbours(body, face) {
        match &body.face(other)?.surface {
            Surface::Plane(p) => {
                if p.frame().z().dot(n).abs() > 1e-12 {
                    return None;
                }
            }
            Surface::Cylinder(c) => {
                if c.frame().z().cross(n).length() > 1e-12 {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(if found.reversed { -n } else { n })
}

/// The letter of a case built, from what was found.
pub fn letter(found: &Found, made: &Body, tol: &Tolerance) -> char {
    if found.invalid {
        'I'
    } else if found.unmeshable {
        'M'
    } else if found.problems.is_empty() {
        'B'
    } else if crosses_itself(made, tol) {
        'X'
    } else {
        'W'
    }
}

/// The cases a scratch run takes, from `HUNT_FROM` to `HUNT_TO` (four when
/// `HUNT_TO` is not set); `None` when `HUNT_FROM` is not set, and the
/// scratch test does nothing.
pub fn scratch_range() -> Option<std::ops::Range<usize>> {
    let from = std::env::var("HUNT_FROM").ok()?.parse().ok()?;
    let to = std::env::var("HUNT_TO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(from + 4);
    Some(from..to)
}

/// One side of a profile: a line, or a cubic Bézier through four points.
#[derive(Clone, Debug)]
pub enum Side {
    Line([f64; 2], [f64; 2]),
    Cubic([[f64; 2]; 4]),
}

impl Side {
    /// The point and the derivative at `t` in [0, 1].
    pub fn at(&self, t: f64) -> ([f64; 2], [f64; 2]) {
        match self {
            Side::Line(a, b) => (
                [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
                [b[0] - a[0], b[1] - a[1]],
            ),
            Side::Cubic(p) => {
                let s = 1.0 - t;
                let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
                let dw = [
                    -3.0 * s * s,
                    3.0 * s * s - 6.0 * s * t,
                    6.0 * s * t - 3.0 * t * t,
                    3.0 * t * t,
                ];
                let mut q = [0.0, 0.0];
                let mut d = [0.0, 0.0];
                for i in 0..4 {
                    q[0] += w[i] * p[i][0];
                    q[1] += w[i] * p[i][1];
                    d[0] += dw[i] * p[i][0];
                    d[1] += dw[i] * p[i][1];
                }
                (q, d)
            }
        }
    }
}

/// Gauss–Legendre on [0, 1], 16 points: exact for polynomials of degree
/// 31, so for every integrand here but the length (a square root), which
/// is summed over 64 pieces.
pub fn gauss() -> Vec<(f64, f64)> {
    // Nodes and weights on [-1, 1] by Newton on the Legendre polynomial.
    let n = 16;
    let mut out = Vec::new();
    for i in 0..n {
        let mut x = (PI * (i as f64 + 0.75) / (n as f64 + 0.5)).pcos();
        let mut dp = 1.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
            let step = p1 / dp;
            x -= step;
            if step.abs() < 1e-16 {
                break;
            }
        }
        let w = 2.0 / ((1.0 - x * x) * dp * dp);
        out.push((0.5 * (x + 1.0), 0.5 * w));
    }
    out
}

/// The profile's area, the first moment of its area about the axis x = 0
/// (`∫∫ x dA`), and the first moment of its boundary's length about it
/// (`∮ x ds`) with the boundary's length.
pub fn pappus(sides: &[Side]) -> (f64, f64, f64, f64) {
    let rule = gauss();
    let (mut area, mut moment, mut line_moment, mut length) = (0.0, 0.0, 0.0, 0.0);
    for side in sides {
        for &(t, w) in &rule {
            let (p, d) = side.at(t);
            area += w * 0.5 * (p[0] * d[1] - p[1] * d[0]);
            moment += w * 0.5 * p[0] * p[0] * d[1];
        }
        let pieces = 64;
        for j in 0..pieces {
            for &(t, w) in &rule {
                let t = (f64::from(j) + t) / f64::from(pieces);
                let (p, d) = side.at(t);
                let ds = d[0].phypot(d[1]) / f64::from(pieces);
                line_moment += w * p[0] * ds;
                length += w * ds;
            }
        }
    }
    (area, moment, line_moment, length)
}

/// A random star-shaped profile about `centre`, counter-clockwise, its
/// sides lines and cubics; with `touch`, its corner farthest left moved
/// onto x = 0.
pub fn profile(rng: &mut Rng, touch: bool, centre: [f64; 2]) -> Vec<Side> {
    let corners = 3 + rng.below(4) as usize;
    let mut points = Vec::new();
    for i in 0..corners {
        let a = TAU * (i as f64 + rng.between(-0.25, 0.25)) / corners as f64;
        let r = rng.between(3.0, 7.0);
        points.push([centre[0] + r * a.pcos(), centre[1] + r * a.psin()]);
    }
    if touch {
        // The corner farthest left moved onto the axis.
        let (i, _) = points
            .iter()
            .enumerate()
            .min_by(|a, b| a.1[0].total_cmp(&b.1[0]))
            .unwrap();
        points[i][0] = 0.0;
    }
    (0..corners)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % corners]);
            if rng.below(2) == 0 {
                Side::Line(a, b)
            } else {
                // Inner control points near the chord, pushed out or in a
                // little across it, so the side stays clear of the others
                // and of the axis.
                let d = [b[0] - a[0], b[1] - a[1]];
                let n = [d[1], -d[0]];
                let (u, v) = (rng.between(-0.2, 0.25), rng.between(-0.2, 0.25));
                let p1 = [a[0] + d[0] / 3.0 + u * n[0], a[1] + d[1] / 3.0 + u * n[1]];
                let p2 = [
                    a[0] + 2.0 * d[0] / 3.0 + v * n[0],
                    a[1] + 2.0 * d[1] / 3.0 + v * n[1],
                ];
                Side::Cubic([a, p1, p2, b])
            }
        })
        .collect()
}

pub fn curves(sides: &[Side], k: f64) -> Vec<SketchCurve> {
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    sides
        .iter()
        .enumerate()
        .map(|(i, side)| SketchCurve {
            id: i as u64 + 1,
            curve: match side {
                Side::Line(a, b) => Curve2::Line {
                    start: s(*a),
                    end: s(*b),
                },
                Side::Cubic(p) => Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: p.iter().map(|q| s(*q)).collect(),
                }),
            },
        })
        .collect()
}
