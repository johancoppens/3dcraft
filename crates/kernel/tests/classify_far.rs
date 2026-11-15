//! The point classifier (`boolean::classify_points`) held to truth known
//! in closed form, and the booleans that stand on it held to volumes
//! known in closed form, to the volume identities and to the same
//! boolean made in the world's frame.
//!
//! The solids: blocks, blocks some of whose sides are ruled strips (flat
//! NURBS faces whose one parameter is a length along the side and the
//! other runs from 0 to 1 across it, as the rim of a plate is ruled
//! between two traced curves; their tangents differ by the size of the
//! model), cylinders, cones, spheres, tori, plates with a rounded outline
//! and a round hole, and unions and cuts of those. Each has a field worked
//! out here, apart from the kernel: negative inside the solid, positive
//! outside it, and in size no more than the distance to its boundary.
//!
//! The points: at 2.5 to a million tolerances to either side of the
//! faces (from a point of the boundary found by halving a segment from
//! inside to outside, along the field's own slope) and in seeded
//! directions off the edges and the vertices; at scales 1e-3, 1 and 1e6;
//! in the world's frame, in turned frames at the origin and up to 5e8
//! model units from it (where a coordinate still holds a sixteenth of the
//! tolerance). At 5e9 a coordinate holds the tolerance no longer: what
//! happens there is counted apart and held to less (`BEYOND`).
//!
//! What is held: a point further from the boundary than twice the
//! tolerance (and what rounding moves it by) is never placed on the wrong
//! side. "On the boundary" within that reach and "unknown" anywhere are
//! not wrong; they are counted.
//!
//! The sweep that runs in the gate takes well under a minute; the wide
//! ones are `#[ignore]`d and run by hand (`--ignored --nocapture`;
//! `SWEEP_SEEDS` seeds per kind, scale and frame).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::Rng;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, check, mass_bound};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

// ---------------------------------------------------------------------
// Fields: negative inside, positive outside, no more than the distance
// ---------------------------------------------------------------------

/// The signed distance from `p` to the box from `lo` to `hi`.
fn box_field(p: Vec3, lo: Vec3, hi: Vec3) -> f64 {
    let (c, h) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
    let q = [
        (p.x - c.x).abs() - h.x,
        (p.y - c.y).abs() - h.y,
        (p.z - c.z).abs() - h.z,
    ];
    let outside = q[0].max(0.0).phypot(q[1].max(0.0)).phypot(q[2].max(0.0));
    outside + q[0].max(q[1]).max(q[2]).min(0.0)
}

/// The signed distance to the cylinder of radius `r` about the vertical
/// through `(cx, cy)` from the level `z0` to `z1`.
fn rod_field(p: Vec3, (cx, cy): (f64, f64), r: f64, (z0, z1): (f64, f64)) -> f64 {
    let side = (p.x - cx).phypot(p.y - cy) - r;
    let ends = (p.z - 0.5 * (z0 + z1)).abs() - 0.5 * (z1 - z0);
    side.max(0.0).phypot(ends.max(0.0)) + side.max(ends).min(0.0)
}

/// The distance from `(x, y)` to the segment `a b`.
fn to_segment((x, y): (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let s = (((x - a.0) * d.0 + (y - a.1) * d.1) / (d.0 * d.0 + d.1 * d.1)).clamp(0.0, 1.0);
    (x - a.0 - s * d.0).phypot(y - a.1 - s * d.1)
}

/// The signed distance to the cone standing on the xy plane with base
/// radius `r` and its apex at height `h`: in the half-plane through the
/// axis the cone is a triangle (mirrored in the axis, which is no part of
/// its boundary).
fn cone_field(p: Vec3, r: f64, h: f64) -> f64 {
    let q = (p.x.phypot(p.y), p.z);
    let d = to_segment(q, (-r, 0.0), (r, 0.0)).min(to_segment(q, (r, 0.0), (0.0, h)));
    let inside = q.1 > 0.0 && q.0 < r * (1.0 - q.1 / h);
    if inside { -d } else { d }
}

fn ball_field(p: Vec3, r: f64) -> f64 {
    p.length() - r
}

fn ring_field(p: Vec3, major: f64, minor: f64) -> f64 {
    (p.x.phypot(p.y) - major).phypot(p.z) - minor
}

/// The signed distance in the plane to the rectangle `w` by `h` about
/// the origin with corners rounded to `r`.
fn rounded_field((x, y): (f64, f64), w: f64, h: f64, r: f64) -> f64 {
    let q = (x.abs() - (0.5 * w - r), y.abs() - (0.5 * h - r));
    q.0.max(0.0).phypot(q.1.max(0.0)) + q.0.max(q.1).min(0.0) - r
}

type Field = Box<dyn Fn(Vec3) -> f64>;

/// Why something could not be made, in words.
trait Why<T> {
    fn why(self, what: &str) -> Result<T, String>;
}

impl<T> Why<T> for Option<T> {
    fn why(self, what: &str) -> Result<T, String> {
        self.ok_or_else(|| what.to_string())
    }
}

impl<T, E: std::fmt::Display> Why<T> for Result<T, E> {
    fn why(self, what: &str) -> Result<T, String> {
        self.map_err(|e| format!("{what}: {e}"))
    }
}

// ---------------------------------------------------------------------
// Solids
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Block,
    /// A block some of whose sides are ruled strips that end where the
    /// side ends.
    Ruled,
    /// The same, the strips reaching past the side as a plate's rim does.
    RuledPast,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    /// A plate of a rounded rectangle with a round hole, by `plate`.
    Plate,
    BlockAndRod,
    BlockLessRod,
    BallLessBlock,
    RingAndBlock,
    RuledLessRod,
    RuledAndBlock,
}

const KINDS: [Kind; 14] = [
    Kind::Block,
    Kind::Ruled,
    Kind::RuledPast,
    Kind::Cylinder,
    Kind::Cone,
    Kind::Sphere,
    Kind::Torus,
    Kind::Plate,
    Kind::BlockAndRod,
    Kind::BlockLessRod,
    Kind::BallLessBlock,
    Kind::RingAndBlock,
    Kind::RuledLessRod,
    Kind::RuledAndBlock,
];

/// A solid in `frame`, its field in the frame's coordinates, and a box
/// (in them) that holds it.
struct Case {
    what: String,
    body: Body,
    frame: Frame,
    field: Field,
    lo: Vec3,
    hi: Vec3,
}

/// The planar faces of `body` chosen by `rng` (at least one) put on flat
/// NURBS surfaces instead: ruled strips, `u` a length along one side of
/// the face and `v` from 0 to 1 across it, or the other way round, or
/// both lengths, or a cubic of uneven spans along `u`; reaching past the
/// face by its own size on every side when `past`. `None` when the body
/// no longer passes the checker.
fn ruled_sides(
    mut body: Body,
    rng: &mut Rng,
    past: bool,
    tol: &Tolerance,
) -> Option<(Body, String)> {
    let mut styles = Vec::new();
    let first = rng.below(body.faces.len() as u64) as usize;
    for index in 0..body.faces.len() {
        if index != first && rng.below(2) == 0 {
            continue;
        }
        let face = &body.faces[index];
        let Surface::Plane(plane) = &face.surface else {
            continue;
        };
        let frame = *plane.frame();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for l in &face.loops {
            for coedge in &body.loop_(*l)?.coedges {
                let edge = body.edge(coedge.edge)?;
                for v in [edge.start, edge.end] {
                    let (x, y, _) = frame.local(body.vertex(v)?.point);
                    lo = [lo[0].min(x), lo[1].min(y)];
                    hi = [hi[0].max(x), hi[1].max(y)];
                }
            }
        }
        if past {
            let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
            lo = [lo[0] - w, lo[1] - h];
            hi = [hi[0] + w, hi[1] + h];
        }
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        let at = |x: f64, y: f64| frame.point(lo[0] + x * w, lo[1] + y * h, 0.0);
        let style = rng.below(5);
        let line = |length: f64| vec![0.0, 0.0, length, length];
        let flat = |ku: Vec<f64>, kv: Vec<f64>| {
            NurbsSurface::new(
                1,
                1,
                ku,
                kv,
                2,
                2,
                vec![at(0.0, 0.0), at(0.0, 1.0), at(1.0, 0.0), at(1.0, 1.0)],
                vec![1.0; 4],
            )
        };
        let surface = match style {
            0 | 1 => flat(line(w), line(1.0)),
            2 => flat(line(1.0), line(h)),
            3 => flat(line(w), line(h)),
            _ => {
                // A cubic along u whose control points stand at the means
                // of three knots (so that u is the length along the side),
                // its spans uneven by a factor of fifty.
                let knots: Vec<f64> = [0.0, 0.0, 0.0, 0.0, 0.01, 0.02, 0.5, 1.0, 1.0, 1.0, 1.0]
                    .iter()
                    .map(|t| t * w)
                    .collect();
                let count = knots.len() - 4;
                let mut ctrl = Vec::new();
                for i in 0..count {
                    let x = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / (3.0 * w);
                    ctrl.push(at(x, 0.0));
                    ctrl.push(at(x, 1.0));
                }
                NurbsSurface::new(3, 1, knots, line(1.0), count, 2, ctrl, vec![1.0; 2 * count])
            }
        };
        body.faces[index].surface = Surface::Nurbs(surface.ok()?);
        styles.push(format!("{index}:{style}"));
    }
    check(&body, tol)
        .is_empty()
        .then(|| (body, styles.join(" ")))
}

/// The sketch of a rounded rectangle `w` by `h` (corners `r`) with a
/// round hole of radius `hole` about `centre`, turned by `turn` and moved
/// to `at`, times `k`.
#[allow(clippy::too_many_arguments)]
fn rounded_sketch(
    w: f64,
    h: f64,
    r: f64,
    centre: (f64, f64),
    hole: f64,
    turn: f64,
    at: (f64, f64),
    k: f64,
) -> Vec<SketchCurve> {
    let (sin, cos) = turn.psin_cos();
    let place = |(x, y): (f64, f64)| {
        [
            at.0 + k * (cos * x - sin * y),
            at.1 + k * (sin * x + cos * y),
        ]
    };
    let (x, y) = (0.5 * w, 0.5 * h);
    // Counter-clockwise from the lower right corner: an arc, then a line.
    let centres = [
        (x - r, -y + r),
        (x - r, y - r),
        (-x + r, y - r),
        (-x + r, -y + r),
    ];
    let from = [(x - r, -y), (x, y - r), (-x + r, y), (-x, -y + r)];
    let to = [(x, -y + r), (x - r, y), (-x, y - r), (-x + r, -y)];
    let mut curves = Vec::new();
    let mut id = 1;
    for i in 0..4 {
        curves.push(SketchCurve {
            id,
            curve: Curve2::Arc {
                center: place(centres[i]),
                start: place(from[i]),
                end: place(to[i]),
            },
        });
        curves.push(SketchCurve {
            id: id + 1,
            curve: Curve2::Line {
                start: place(to[i]),
                end: place(from[(i + 1) % 4]),
            },
        });
        id += 2;
    }
    curves.push(SketchCurve {
        id,
        curve: Curve2::Circle {
            center: place(centre),
            radius: hole * k,
        },
    });
    curves
}

/// A solid of `kind` at scale `k` in `frame`, sized by `rng`, or why it
/// could not be built (counted by the caller).
fn make(kind: Kind, rng: &mut Rng, k: f64, frame: &Frame) -> Result<Case, String> {
    make_with(kind, rng, k, frame).map_err(|why| shortened(&why))
}

/// The first words of a reason, without its numbers.
fn shortened(why: &str) -> String {
    let words: String = why
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    words.chars().take(110).collect()
}

const UNRULED: &str = "ruled sides that fail the checker";

fn make_with(kind: Kind, rng: &mut Rng, k: f64, frame: &Frame) -> Result<Case, String> {
    let tol = tolerance(k);
    let v = |x: f64, y: f64, z: f64| Vec3::new(x * k, y * k, z * k);
    let block_at = |lo: Vec3, size: Vec3| {
        let f = frame.at(frame.point(lo.x, lo.y, lo.z))?;
        primitives::block(&f, size.x, size.y, size.z).ok()
    };
    let rod_at = |cx: f64, cy: f64, z0: f64, r: f64, height: f64| {
        let f = frame.at(frame.point(cx, cy, z0))?;
        primitives::cylinder(&f, r, height).ok()
    };
    let (a, b, c) = (
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
    );
    let size = v(a, b, c);
    let zero = Vec3::new(0.0, 0.0, 0.0);
    let made = |what: String, body: Body, field: Field, lo: Vec3, hi: Vec3| {
        Ok(Case {
            what,
            body,
            frame: *frame,
            field,
            lo,
            hi,
        })
    };
    let block_field = move |p: Vec3| box_field(p, zero, size);
    // A rod through the block, well inside its outline.
    let (r, cx, cy) = {
        let r = rng.between(0.1, 0.3) * a.min(b);
        (
            r * k,
            rng.between(1.2 * r, a - 1.2 * r) * k,
            rng.between(1.2 * r, b - 1.2 * r) * k,
        )
    };
    match kind {
        Kind::Block => made(
            format!("block {a:.3} x {b:.3} x {c:.3}"),
            block_at(zero, size).why("a block")?,
            Box::new(block_field),
            zero,
            size,
        ),
        Kind::Ruled | Kind::RuledPast => {
            let past = kind == Kind::RuledPast;
            let block = block_at(zero, size).why("a block")?;
            let (body, styles) = ruled_sides(block, rng, past, &tol).why(UNRULED)?;
            made(
                format!("block {a:.3} x {b:.3} x {c:.3} ruled [{styles}]"),
                body,
                Box::new(block_field),
                zero,
                size,
            )
        }
        Kind::Cylinder => {
            let (r, h) = (rng.between(5.0, 25.0) * k, c * k);
            made(
                format!("cylinder r {r:e} h {h:e}"),
                primitives::cylinder(frame, r, h).why("cylinder")?,
                Box::new(move |p| rod_field(p, (0.0, 0.0), r, (0.0, h))),
                Vec3::new(-r, -r, 0.0),
                Vec3::new(r, r, h),
            )
        }
        Kind::Cone => {
            let (r, h) = (rng.between(5.0, 25.0) * k, c * k);
            made(
                format!("cone r {r:e} h {h:e}"),
                primitives::cone(frame, r, h).why("cone")?,
                Box::new(move |p| cone_field(p, r, h)),
                Vec3::new(-r, -r, 0.0),
                Vec3::new(r, r, h),
            )
        }
        Kind::Sphere => {
            let r = rng.between(5.0, 30.0) * k;
            made(
                format!("sphere r {r:e}"),
                primitives::sphere(frame, r).why("sphere")?,
                Box::new(move |p| ball_field(p, r)),
                Vec3::new(-r, -r, -r),
                Vec3::new(r, r, r),
            )
        }
        Kind::Torus => {
            let major = rng.between(15.0, 40.0) * k;
            let minor = rng.between(0.1, 0.6) * major;
            made(
                format!("torus {major:e} {minor:e}"),
                primitives::torus(frame, major, minor).why("torus")?,
                Box::new(move |p| ring_field(p, major, minor)),
                Vec3::new(-major - minor, -major - minor, -minor),
                Vec3::new(major + minor, major + minor, minor),
            )
        }
        Kind::Plate => {
            let (w, h) = (rng.between(30.0, 80.0), rng.between(20.0, 60.0));
            let m = 0.5 * w.min(h);
            let corner = if rng.below(3) == 0 {
                // Near the most it can be: two sides of almost no length.
                m * (1.0 - 10f64.ppowf(-rng.between(2.0, 5.0)))
            } else {
                rng.between(0.3, m - 0.3)
            };
            let hole = rng.between(0.1, 0.3) * m;
            let centre = (rng.between(-0.2, 0.2) * m, rng.between(-0.2, 0.2) * m);
            let turn = rng.between(0.0, TAU);
            let at = (
                rng.between(-100.0, 100.0) * k,
                rng.between(-100.0, 100.0) * k,
            );
            let t = 10f64.ppowf(rng.between(-1.3, 1.0)) * k;
            let curves = rounded_sketch(w, h, corner, centre, hole, turn, at, k);
            let profiles = detect_profiles(&curves, &tol).why("profiles")?;
            let sheet = fill(&profiles, frame, &tol).why("fill")?.body;
            let body = plate(&sheet, &[FaceId(0)], t, Side::Mid, &BTreeMap::new(), &tol)
                .why("plate")?
                .body;
            let (sin, cos) = turn.psin_cos();
            let field = move |p: Vec3| {
                let (dx, dy) = (p.x - at.0, p.y - at.1);
                let (x, y) = ((cos * dx + sin * dy) / k, (-sin * dx + cos * dy) / k);
                let outline = rounded_field((x, y), w, h, corner)
                    .max(hole - (x - centre.0).phypot(y - centre.1))
                    * k;
                let skins = p.z.abs() - 0.5 * t;
                outline.max(0.0).phypot(skins.max(0.0)) + outline.max(skins).min(0.0)
            };
            let reach = 0.5 * w.phypot(h) * k;
            made(
                format!(
                    "plate {w:.3} x {h:.3} r {corner:.6} hole {hole:.3} t {:.4} turned {turn:.4}",
                    t / k
                ),
                body,
                Box::new(field),
                Vec3::new(at.0 - reach, at.1 - reach, -0.5 * t),
                Vec3::new(at.0 + reach, at.1 + reach, 0.5 * t),
            )
        }
        Kind::BlockAndRod | Kind::BlockLessRod | Kind::RuledLessRod => {
            let block = block_at(zero, size).why("a block")?;
            let (block, styles) = if kind == Kind::RuledLessRod {
                ruled_sides(block, rng, false, &tol).why(UNRULED)?
            } else {
                (block, String::new())
            };
            // Standing in the block and out of its top, or through it.
            let (z0, z1, op) = if kind == Kind::BlockAndRod {
                (0.5 * size.z, 1.5 * size.z, BooleanOp::Union)
            } else {
                (-0.25 * size.z, 1.25 * size.z, BooleanOp::Subtract)
            };
            let rod = rod_at(cx, cy, z0, r, z1 - z0).why("a rod")?;
            let body = boolean(&block, &rod, op, &tol).why("boolean")?.body;
            let field: Field = if op == BooleanOp::Union {
                Box::new(move |p| block_field(p).min(rod_field(p, (cx, cy), r, (z0, z1))))
            } else {
                Box::new(move |p| block_field(p).max(-rod_field(p, (cx, cy), r, (z0, z1))))
            };
            made(
                format!(
                    "block {a:.3} x {b:.3} x {c:.3} [{styles}] {op:?} rod r {r:e} at ({cx:e}, {cy:e})"
                ),
                body,
                field,
                Vec3::new(0.0, 0.0, z0.min(0.0)),
                Vec3::new(size.x, size.y, z1.max(size.z)),
            )
        }
        Kind::BallLessBlock => {
            let r = rng.between(5.0, 30.0) * k;
            let ball = primitives::sphere(frame, r).why("sphere")?;
            let bite = Vec3::new(2.0 * r, 2.0 * r, 2.0 * r);
            let block = block_at(zero, bite).why("a block")?;
            let body = boolean(&ball, &block, BooleanOp::Subtract, &tol)
                .why("boolean")?
                .body;
            made(
                format!("sphere r {r:e} less an octant"),
                body,
                Box::new(move |p| ball_field(p, r).max(-box_field(p, zero, bite))),
                Vec3::new(-r, -r, -r),
                Vec3::new(r, r, r),
            )
        }
        Kind::RingAndBlock => {
            let major = rng.between(15.0, 40.0) * k;
            let minor = rng.between(0.15, 0.5) * major;
            let ring = primitives::torus(frame, major, minor).why("torus")?;
            // A bar through the ring's hole and both sides of its tube.
            let lo = Vec3::new(-(major + 2.0 * minor), -0.5 * minor, -0.5 * minor);
            let hi = Vec3::new(major + 2.0 * minor, 0.5 * minor, 0.5 * minor);
            let bar = block_at(lo, hi - lo).why("a block")?;
            let body = boolean(&ring, &bar, BooleanOp::Union, &tol)
                .why("boolean")?
                .body;
            made(
                format!("torus {major:e} {minor:e} and a bar"),
                body,
                Box::new(move |p| ring_field(p, major, minor).min(box_field(p, lo, hi))),
                Vec3::new(-(major + 2.0 * minor), -major - minor, -minor),
                Vec3::new(major + 2.0 * minor, major + minor, minor),
            )
        }
        Kind::RuledAndBlock => {
            let block = block_at(zero, size).why("a block")?;
            let (block, styles) = ruled_sides(block, rng, false, &tol).why(UNRULED)?;
            let lo = size * 0.5;
            let other = size * 0.8;
            let second = block_at(lo, other).why("a block")?;
            let body = boolean(&block, &second, BooleanOp::Union, &tol)
                .why("boolean")?
                .body;
            made(
                format!(
                    "block {a:.3} x {b:.3} x {c:.3} ruled [{styles}] and a block at its middle"
                ),
                body,
                Box::new(move |p| block_field(p).min(box_field(p, lo, lo + other))),
                zero,
                lo + other,
            )
        }
    }
}

// ---------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------

/// The frames a solid is placed in: its name, whether its axes are
/// turned, and how far from the origin it is in model units at scale 1.
const FRAMES: [(&str, bool, f64); 7] = [
    ("world", false, 0.0),
    ("turned", true, 0.0),
    ("turned 5e2", true, 5e2),
    ("turned 5e4", true, 5e4),
    ("turned 5e6", true, 5e6),
    ("world 5e6", false, 5e6),
    ("turned 5e8", true, 5e8),
];

/// Where a coordinate no longer holds the tolerance (its last bit is one
/// tolerance at scale 1): counted apart.
const BEYOND: (&str, bool, f64) = ("turned 5e9", true, 5e9);

fn placed(rng: &mut Rng, k: f64, (_, turned, far): (&str, bool, f64)) -> Frame {
    let random = |rng: &mut Rng| {
        Vec3::new(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        )
    };
    let (z, x, away) = (random(rng), random(rng), random(rng));
    let origin = away.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0)) * (far * k);
    if turned {
        Frame::new(origin, z, x).unwrap_or(Frame::WORLD)
    } else {
        Frame::WORLD.at(origin).unwrap_or(Frame::WORLD)
    }
}

// ---------------------------------------------------------------------
// Points and the count
// ---------------------------------------------------------------------

/// How far from the boundary the points are put, in tolerances.
const OFFSETS: [f64; 8] = [2.5, 4.0, 10.0, 1e2, 1e3, 1e4, 1e5, 1e6];

/// The distances the count is kept by, in tolerances: within reach of
/// the boundary (any answer), then up to each of these, then beyond.
const BINS: [&str; 5] = ["at the boundary", "to 10", "to 1e2", "to 1e4", "over 1e4"];

fn bin(off: f64) -> usize {
    match off {
        o if o <= 10.0 => 1,
        o if o <= 1e2 => 2,
        o if o <= 1e4 => 3,
        _ => 4,
    }
}

/// How far from the boundary, in tolerances, the classifier may still
/// call a point on it: within twice the tolerance of a face's surface
/// with its foot within the tolerance of the face, which beside an edge
/// is `√(2² + 1²)` from the edge.
const ON_REACH: f64 = 2.2361;

/// Right, on the boundary, unknown, wrong.
type Counts = [usize; 4];

#[derive(Default)]
struct Sweep {
    by_bin: BTreeMap<usize, Counts>,
    by_scale: BTreeMap<String, Counts>,
    by_frame: BTreeMap<String, Counts>,
    by_kind: BTreeMap<String, Counts>,
    /// Solids that could not be set up, by kind and frame.
    not_made: BTreeMap<String, usize>,
    made: usize,
    wrong: Vec<String>,
    /// Points called on the boundary from beyond its reach.
    on_far: Vec<String>,
    seconds: BTreeMap<String, f64>,
}

impl Sweep {
    fn add(&mut self, bin: usize, scale: f64, frame: &str, kind: Kind, answer: usize) {
        self.by_bin.entry(bin).or_default()[answer] += 1;
        // Within reach of the boundary any answer stands: the other
        // tables count the points beyond it.
        if bin == 0 {
            return;
        }
        self.by_scale.entry(format!("{scale:e}")).or_default()[answer] += 1;
        self.by_frame.entry(frame.to_string()).or_default()[answer] += 1;
        self.by_kind.entry(format!("{kind:?}")).or_default()[answer] += 1;
    }

    fn wrong(&self) -> usize {
        self.by_bin
            .iter()
            .filter(|(bin, _)| **bin > 0)
            .map(|(_, c)| c[3])
            .sum()
    }

    fn table(&self, name: &str) -> String {
        let mut out = format!(
            "{name}: {} solids, {} not set up {:?}\n",
            self.made,
            self.not_made.values().sum::<usize>(),
            self.not_made
        );
        let row = |label: &str, c: &Counts| {
            format!(
                "  {label:<18} right {:>8}  on {:>6}  unknown {:>6}  WRONG {:>6}\n",
                c[0], c[1], c[2], c[3]
            )
        };
        out += " by distance from the boundary, in tolerances\n";
        for (bin, c) in &self.by_bin {
            out += &row(BINS[*bin], c);
        }
        for (title, map) in [
            (" by scale (beyond reach of the boundary)", &self.by_scale),
            (" by frame", &self.by_frame),
            (" by kind", &self.by_kind),
        ] {
            out += title;
            out += "\n";
            for (label, c) in map {
                out += &row(label, c);
            }
        }
        out += " seconds classifying, by kind:";
        for (kind, s) in &self.seconds {
            out += &format!(" {kind} {s:.2}");
        }
        out += "\n";
        for line in self.wrong.iter().take(12) {
            out += &format!("  WRONG {line}\n");
        }
        for line in self.on_far.iter().take(6) {
            out += &format!("  on the boundary from beyond its reach: {line}\n");
        }
        out
    }
}

/// The points to classify about `case`, in the world: off the faces
/// along the field's slope, off the edges and the vertices in seeded
/// directions.
fn points(case: &Case, rng: &mut Rng, tol: f64, features: usize) -> Vec<Vec3> {
    let field = |q: Vec3| (case.field)(q);
    let extent = (case.hi - case.lo).length();
    let margin = (case.hi - case.lo) * 0.2;
    let (lo, hi) = (case.lo - margin, case.hi + margin);
    let random_in = |rng: &mut Rng| {
        Vec3::new(
            rng.between(lo.x, hi.x),
            rng.between(lo.y, hi.y),
            rng.between(lo.z, hi.z),
        )
    };
    // Points inside and outside, to halve between.
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    for _ in 0..4000 {
        if inside.len() >= features && outside.len() >= features {
            break;
        }
        let q = random_in(rng);
        if field(q) < 0.0 {
            if inside.len() < features {
                inside.push(q);
            }
        } else if outside.len() < features {
            outside.push(q);
        }
    }
    let mut local = Vec::new();
    for (a, b) in inside.iter().zip(&outside) {
        let (mut a, mut b) = (*a, *b);
        for _ in 0..80 {
            let mid = (a + b) * 0.5;
            if field(mid) < 0.0 {
                a = mid;
            } else {
                b = mid;
            }
        }
        let q = (a + b) * 0.5;
        // The field's slope there: of length one off a face, and its
        // outward normal; less at an edge, where the offsets are taken
        // in seeded directions instead.
        let h = 1e-6 * extent;
        let slope = Vec3::new(
            field(q + Vec3::new(h, 0.0, 0.0)) - field(q - Vec3::new(h, 0.0, 0.0)),
            field(q + Vec3::new(0.0, h, 0.0)) - field(q - Vec3::new(0.0, h, 0.0)),
            field(q + Vec3::new(0.0, 0.0, h)) - field(q - Vec3::new(0.0, 0.0, h)),
        ) * (0.5 / h);
        let normal = slope
            .normalized()
            .filter(|_| (slope.length() - 1.0).abs() < 1e-3);
        for offset in OFFSETS {
            match normal {
                Some(n) => {
                    local.push(q + n * (offset * tol));
                    local.push(q - n * (offset * tol));
                }
                None => {
                    for _ in 0..2 {
                        local.push(q + unit(rng) * (offset * tol));
                    }
                }
            }
        }
    }
    let mut world: Vec<Vec3> = local
        .into_iter()
        .map(|q| case.frame.point(q.x, q.y, q.z))
        .collect();
    // Off the vertices and off two points of each edge.
    let mut corners: Vec<Vec3> = case.body.vertices.iter().map(|v| v.point).collect();
    for edge in &case.body.edges {
        for s in [0.3, 0.7] {
            corners.push(
                edge.curve
                    .eval(edge.range.0 + s * (edge.range.1 - edge.range.0)),
            );
        }
    }
    for _ in 0..features.min(corners.len()) {
        let q = corners[rng.below(corners.len() as u64) as usize];
        for offset in OFFSETS {
            for _ in 0..2 {
                world.push(q + unit(rng) * (offset * tol));
            }
        }
    }
    world
}

fn unit(rng: &mut Rng) -> Vec3 {
    loop {
        let v = Vec3::new(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        );
        if let Some(u) = v.normalized().filter(|_| v.length() <= 1.0) {
            return u;
        }
    }
}

/// One solid: its points classified and counted against its field.
fn count(
    sweep: &mut Sweep,
    case: &Case,
    kind: Kind,
    k: f64,
    frame: &str,
    rng: &mut Rng,
    features: usize,
) {
    let tol = tolerance(k);
    let points = points(case, rng, tol.linear, features);
    let started = std::time::Instant::now();
    let places = match classify_points(&case.body, &points, &tol) {
        Ok(places) => places,
        Err(e) => {
            *sweep
                .not_made
                .entry(format!("{kind:?} in {frame}: not classified: {e}"))
                .or_default() += 1;
            return;
        }
    };
    *sweep.seconds.entry(format!("{kind:?}")).or_default() += started.elapsed().as_secs_f64();
    sweep.made += 1;
    for (p, place) in points.iter().zip(places) {
        let (x, y, z) = case.frame.local(*p);
        let truth = (case.field)(Vec3::new(x, y, z));
        // What rounding moves the point and the field by: thirty-two
        // last bits of its largest coordinate.
        let size = p.x.abs().max(p.y.abs()).max(p.z.abs());
        let reach = ON_REACH * tol.linear + 32.0 * f64::EPSILON * size;
        let bin = if truth.abs() <= reach {
            0
        } else {
            bin(truth.abs() / tol.linear)
        };
        let answer = match place {
            Place::Inside if truth < 0.0 => 0,
            Place::Outside if truth >= 0.0 => 0,
            Place::OnBoundary => 1,
            Place::Unknown => 2,
            Place::Inside | Place::Outside => 3,
        };
        if answer == 1 && bin > 0 {
            sweep.on_far.push(format!(
                "{} at scale {k:e} in {frame}: at {p:?}, {:.4} tolerances from the boundary",
                case.what,
                truth.abs() / tol.linear
            ));
        }
        if answer == 3 && bin > 0 {
            sweep.wrong.push(format!(
                "{} at scale {k:e} in {frame}: {place:?} at {p:?}, {:.4e} tolerances {} the boundary",
                case.what,
                truth.abs() / tol.linear,
                if truth < 0.0 { "inside" } else { "outside" }
            ));
        }
        sweep.add(bin, k, frame, kind, answer);
    }
}

fn sweep(seeds: std::ops::Range<u64>, frames: &[(&str, bool, f64)], features: usize) -> Sweep {
    let mut sweep = Sweep::default();
    for seed in seeds {
        for (ki, kind) in KINDS.iter().enumerate() {
            for (si, k) in SCALES.iter().enumerate() {
                for (fi, frame) in frames.iter().enumerate() {
                    let mut rng = Rng(0xC1A5_51F7
                        ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                        ^ ((ki as u64) << 40)
                        ^ ((si as u64) << 32)
                        ^ ((fi as u64) << 24));
                    let f = placed(&mut rng, *k, *frame);
                    match make(*kind, &mut rng, *k, &f) {
                        Ok(case) => {
                            count(&mut sweep, &case, *kind, *k, frame.0, &mut rng, features)
                        }
                        Err(why) => {
                            *sweep
                                .not_made
                                .entry(format!("{kind:?} in {}: {why}", frame.0))
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    sweep
}

fn seeds_asked(default: u64) -> std::ops::Range<u64> {
    let from = std::env::var("SWEEP_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let seeds = std::env::var("SWEEP_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    from..from + seeds
}

/// No point further from the boundary than twice the tolerance (and
/// rounding) is placed on the wrong side: every kind, scale and frame
/// where a coordinate holds the tolerance.
#[test]
fn a_point_clear_of_the_boundary_is_never_placed_on_the_wrong_side() {
    let found = sweep(0..2, &FRAMES, 4);
    println!("{}", found.table("classify sweep"));
    assert!(found.made >= 500, "{} solids made", found.made);
    assert_eq!(found.wrong(), 0, "\n{}", found.table("classify sweep"));
}

/// The wide sweep, by hand: `SWEEP_SEEDS` seeds (default 12) from
/// `SWEEP_FROM`.
#[test]
#[ignore = "a wide sweep, run by hand"]
fn a_point_clear_of_the_boundary_wide() {
    let found = sweep(seeds_asked(12), &FRAMES, 8);
    println!("{}", found.table("classify sweep (wide)"));
    assert_eq!(
        found.wrong(),
        0,
        "\n{}",
        found.table("classify sweep (wide)")
    );
}

/// Where a coordinate no longer holds the tolerance (5e9 model units
/// from the origin at scale 1: its last bit is a tolerance), a solid may
/// not be built and a point near the boundary may be placed anywhere;
/// counted, and only a point more than ten thousand tolerances from the
/// boundary is held to its side.
#[test]
#[ignore = "a measure beyond what the coordinates hold, run by hand"]
fn beyond_what_the_coordinates_hold() {
    let found = sweep(seeds_asked(4), &[BEYOND], 8);
    println!("{}", found.table("classify sweep (beyond)"));
    let far_wrong = found.by_bin.get(&4).map_or(0, |c| c[3]);
    assert_eq!(far_wrong, 0, "\n{}", found.table("classify sweep (beyond)"));
}

/// The smallest case: a block of 9e3 by 2e7 by 2e7 at tolerance 1, one
/// side a flat strip ruled across it (`u` the length along its 9e3, `v`
/// from 0 to 1 across its 2e7). Points 6600 tolerances outside that side
/// and inside it, in the frame of `hunt_sheet_plates` case 123 and in
/// the world: the first ray from a point outside crosses the strip (a
/// crossing that was left out: the point was placed inside) and leaves
/// through the top.
#[test]
fn a_point_beside_a_side_ruled_across_millions_is_placed_as_it_lies() {
    let tol = tolerance(1e6);
    let (length, width, height) = (9e3, 2e7, 2e7);
    for (name, frame) in [
        ("the world", Frame::WORLD),
        (
            "turned and far",
            Frame::new(
                Vec3::new(-4.7e7, -1.7e8, -1.7e8),
                Vec3::new(0.2304, -0.7257, 0.6482),
                Vec3::new(-0.5001, 0.4832, 0.7187),
            )
            .unwrap(),
        ),
    ] {
        let mut body = primitives::block(&frame, length, width, height).unwrap();
        // The side in the plane y = 0 of the frame.
        let index = body
            .faces
            .iter()
            .position(|f| match &f.surface {
                Surface::Plane(p) => {
                    p.frame().z().dot(frame.y()).abs() > 0.999_999
                        && frame.local(p.frame().origin()).1.abs() < 1.0
                }
                _ => false,
            })
            .unwrap();
        // u along the frame's x, v across its z: the strip's normal is
        // the side's outward one, against the frame's y.
        let corner = |x: f64, z: f64| frame.point(x * length, 0.0, z * height);
        let strip = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, length, length],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![
                corner(0.0, 0.0),
                corner(0.0, 1.0),
                corner(1.0, 0.0),
                corner(1.0, 1.0),
            ],
            vec![1.0; 4],
        )
        .unwrap();
        body.faces[index].surface = Surface::Nurbs(strip);
        body.faces[index].reversed = false;
        assert_eq!(check(&body, &tol), [], "{name}");
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for (x, z) in [
            (0.47, 0.5),
            (0.31, 0.27),
            (0.06, 0.69),
            (0.9, 0.5),
            (0.5, 0.03),
        ] {
            for (off, place) in [(-6600.0, Place::Outside), (6600.0, Place::Inside)] {
                points.push(frame.point(x * length, off, z * height));
                expected.push(place);
            }
        }
        let places = classify_points(&body, &points, &tol).unwrap();
        assert_eq!(places, expected, "{name}");
    }
}

// ---------------------------------------------------------------------
// The booleans that stand on it
// ---------------------------------------------------------------------

/// What happened to one boolean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Right,
    Refused,
    /// Built and passing the checker, and not the result.
    Wrong,
    /// Built and failing the checker or the measure.
    Invalid,
}

/// Two solids that overlap, in `frame` at scale `k`: their name, the two
/// bodies and, where there is one, the closed form of the volume they
/// share.
fn pair(
    which: u64,
    rng: &mut Rng,
    k: f64,
    frame: &Frame,
) -> Option<(String, Body, Body, Option<f64>)> {
    let tol = tolerance(k);
    let block_at = |lo: Vec3, size: Vec3, f: &Frame| {
        let f = f.at(f.point(lo.x, lo.y, lo.z))?;
        primitives::block(&f, size.x, size.y, size.z).ok()
    };
    let zero = Vec3::new(0.0, 0.0, 0.0);
    let size = Vec3::new(
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
    ) * k;
    // A second block that shares a corner region of the first.
    let lo = Vec3::new(
        rng.between(0.2, 0.8) * size.x,
        rng.between(0.2, 0.8) * size.y,
        rng.between(0.2, 0.8) * size.z,
    );
    let other = Vec3::new(
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
        rng.between(8.0, 60.0),
    ) * k;
    let shared = |lo: Vec3, other: Vec3| {
        (size.x - lo.x).min(other.x) * (size.y - lo.y).min(other.y) * (size.z - lo.z).min(other.z)
    };
    let first = block_at(zero, size, frame)?;
    let rod = |rng: &mut Rng| {
        let r = rng.between(0.1, 0.3) * size.x.min(size.y);
        let (cx, cy) = (
            rng.between(1.2 * r, size.x - 1.2 * r),
            rng.between(1.2 * r, size.y - 1.2 * r),
        );
        let f = frame.at(frame.point(cx, cy, -0.25 * size.z))?;
        let body = primitives::cylinder(&f, r, 1.5 * size.z).ok()?;
        Some((body, std::f64::consts::PI * r * r * size.z))
    };
    match which {
        0 => Some((
            "two blocks".into(),
            first,
            block_at(lo, other, frame)?,
            Some(shared(lo, other)),
        )),
        1 => {
            let (ruled, styles) = ruled_sides(first, rng, false, &tol)?;
            Some((
                format!(
                    "a ruled block {:?} [{styles}] and a block {:?} at {:?}",
                    size * (1.0 / k),
                    other * (1.0 / k),
                    lo * (1.0 / k)
                ),
                ruled,
                block_at(lo, other, frame)?,
                Some(shared(lo, other)),
            ))
        }
        2 => {
            let (ruled, styles) = ruled_sides(first, rng, true, &tol)?;
            let (second, styles_b) = ruled_sides(block_at(lo, other, frame)?, rng, false, &tol)?;
            Some((
                format!("a ruled block [{styles}] and a ruled block [{styles_b}]"),
                ruled,
                second,
                Some(shared(lo, other)),
            ))
        }
        3 => {
            // The second block turned about the corner it starts at.
            let (ruled, styles) = ruled_sides(first, rng, false, &tol)?;
            let turned = Frame::new(
                frame.point(lo.x, lo.y, lo.z),
                frame.vector(rng.between(-0.4, 0.4), rng.between(-0.4, 0.4), 1.0),
                frame.vector(1.0, rng.between(-0.4, 0.4), rng.between(-0.4, 0.4)),
            )?;
            let second = primitives::block(&turned, other.x, other.y, other.z).ok()?;
            Some((
                format!("a ruled block [{styles}] and a turned block"),
                ruled,
                second,
                None,
            ))
        }
        4 => {
            let (ruled, styles) = ruled_sides(first, rng, false, &tol)?;
            let (rod, through) = rod(rng)?;
            Some((
                format!("a ruled block [{styles}] and a rod through it"),
                ruled,
                rod,
                Some(through),
            ))
        }
        5 => {
            let (rod, through) = rod(rng)?;
            Some((
                "a block and a rod through it".into(),
                first,
                rod,
                Some(through),
            ))
        }
        6 => {
            let r = rng.between(0.3, 0.6) * size.x.min(size.y).min(size.z);
            let f = frame.at(frame.point(lo.x, lo.y, lo.z))?;
            let (ruled, styles) = ruled_sides(first, rng, false, &tol)?;
            Some((
                format!("a ruled block [{styles}] and a ball"),
                ruled,
                primitives::sphere(&f, r).ok()?,
                None,
            ))
        }
        _ => {
            // A plate (its rims ruled where they are not planes or
            // cylinders) and a block through its outline.
            let (w, h) = (rng.between(30.0, 80.0), rng.between(20.0, 60.0));
            let m = 0.5 * w.min(h);
            let corner = m * (1.0 - 10f64.ppowf(-rng.between(2.0, 5.0)));
            let t = 10f64.ppowf(rng.between(0.3, 1.0)) * k;
            let turn = rng.between(0.0, TAU);
            let curves = rounded_sketch(w, h, corner, (0.0, 0.0), 0.2 * m, turn, (0.0, 0.0), k);
            let profiles = detect_profiles(&curves, &tol).ok()?;
            let sheet = fill(&profiles, frame, &tol).ok()?.body;
            let plated = plate(&sheet, &[FaceId(0)], t, Side::Mid, &BTreeMap::new(), &tol)
                .ok()?
                .body;
            // Through an end of the outline, both skins and the rim.
            let (sin, cos) = turn.psin_cos();
            let end = (0.5 * w.max(h)) * k;
            let (ex, ey) = if w >= h {
                (cos * end, sin * end)
            } else {
                (-sin * end, cos * end)
            };
            let cutter = Frame::new(
                frame.point(ex, ey, -2.0 * t),
                frame.z(),
                frame.vector(cos, sin, 0.0),
            )?;
            let half = 0.3 * m * k;
            let cutter = cutter.at(cutter.point(-half, -half, 0.0))?;
            let block = primitives::block(&cutter, 2.0 * half, 2.0 * half, 4.0 * t).ok()?;
            Some((
                format!(
                    "a plate {w:.3} x {h:.3} r {corner:.6} t {:.3} and a block through its end",
                    t / k
                ),
                plated,
                block,
                None,
            ))
        }
    }
}

const PAIRS: u64 = 8;
const PAIR_NAMES: [&str; 8] = [
    "two blocks",
    "a ruled block and a block",
    "two ruled blocks",
    "a ruled block and a turned block",
    "a ruled block and a rod through it",
    "a block and a rod through it",
    "a ruled block and a ball",
    "a plate and a block through its end",
];

struct Volumes {
    /// Union, intersection, subtraction: the volume and its allowance,
    /// or why there is none.
    ops: [Result<(f64, f64), Outcome>; 3],
    a: (f64, f64),
    b: (f64, f64),
    area: f64,
    /// Why the booleans that were refused were.
    refusals: Vec<String>,
}

fn volumes(a: &Body, b: &Body, tol: &Tolerance) -> Option<Volumes> {
    let measure = |body: &Body| {
        let (m, bound) = mass_bound(body).ok()?;
        (m.volume.is_finite() && bound.volume.is_finite()).then_some((
            m.volume,
            bound.volume,
            m.area,
        ))
    };
    let (va, ba, aa) = measure(a)?;
    let (vb, bb, ab) = measure(b)?;
    let refusals = std::cell::RefCell::new(Vec::new());
    let op = |op: BooleanOp| match boolean(a, b, op, tol) {
        Err(e) => {
            refusals.borrow_mut().push(shortened(&e.to_string()));
            Err(Outcome::Refused)
        }
        Ok(result) => {
            if !check(&result.body, tol).is_empty() {
                return Err(Outcome::Invalid);
            }
            match measure(&result.body) {
                Some((v, bound, _)) => Ok((v, bound)),
                None => Err(Outcome::Invalid),
            }
        }
    };
    let ops = [
        op(BooleanOp::Union),
        op(BooleanOp::Intersect),
        op(BooleanOp::Subtract),
    ];
    Some(Volumes {
        ops,
        a: (va, ba),
        b: (vb, bb),
        area: aa + ab,
        refusals: refusals.into_inner(),
    })
}

#[derive(Default)]
struct Booleans {
    /// Per kind of pair and scale: right, refused, wrong, invalid.
    counts: BTreeMap<String, [usize; 4]>,
    not_made: usize,
    wrong: Vec<String>,
    /// Why booleans were refused, and how many for each reason.
    refusals: BTreeMap<String, usize>,
}

impl Booleans {
    fn table(&self, name: &str) -> String {
        let mut out = format!("{name}: {} pairs not set up\n", self.not_made);
        let mut all = [0; 4];
        for (label, c) in &self.counts {
            out += &format!(
                "  {label:<58} right {:>5}  refused {:>5}  WRONG {:>5}  invalid {:>5}\n",
                c[0], c[1], c[2], c[3]
            );
            for (sum, n) in all.iter_mut().zip(c) {
                *sum += n;
            }
        }
        out += &format!(
            "  {:<58} right {:>5}  refused {:>5}  WRONG {:>5}  invalid {:>5}\n",
            "all", all[0], all[1], all[2], all[3]
        );
        for line in self.wrong.iter().take(16) {
            out += &format!("  WRONG {line}\n");
        }
        for (why, n) in &self.refusals {
            out += &format!("  refused {n} times: {why}\n");
        }
        out
    }

    fn wrong(&self) -> usize {
        self.counts.values().map(|c| c[2] + c[3]).sum()
    }
}

/// What the three booleans of one seeded pair came to.
struct Judged {
    what: String,
    frame: Frame,
    /// Union, intersection, subtraction.
    verdicts: [Outcome; 3],
    /// What shows a boolean wrong.
    why: Vec<String>,
    refusals: Vec<String>,
}

/// The pair `which` of `seed` at scale `k`, in a turned frame `far` model
/// units (at scale 1) from the origin and in the world: every boolean of
/// the three held to the closed form where there is one, to the two
/// identities (`V(A ∪ B) + V(A ∩ B) = V(A) + V(B)`,
/// `V(A − B) + V(A ∩ B) = V(A)`) and to the same boolean of the same
/// solids in the world. `None` when the pair cannot be set up.
fn judged(seed: u64, which: u64, k: f64, far: f64) -> Option<Judged> {
    let rng_at = || {
        Rng(0xB001_EA75 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (which << 40) ^ k.to_bits())
    };
    let tol = tolerance(k);
    // The frame's own numbers come from another stream, so that the
    // solids are the same in the world.
    let mut frame_rng = Rng(rng_at().0 ^ 0x5EED);
    let frame = placed(&mut frame_rng, k, ("", true, far));
    let (what, a, b, closed) = pair(which, &mut rng_at(), k, &frame)?;
    let (_, wa, wb, _) = pair(which, &mut rng_at(), k, &Frame::WORLD)?;
    let (here, world) = (volumes(&a, &b, &tol)?, volumes(&wa, &wb, &tol)?);
    let slack = |bounds: f64| bounds + 4.0 * tol.linear * here.area + 1e-9 * (here.a.0 + here.b.0);
    // What each volume must be, by the closed form.
    let expected = closed.map(|shared| [here.a.0 + here.b.0 - shared, shared, here.a.0 - shared]);
    let names = ["union", "intersection", "subtraction"];
    let mut verdicts = [Outcome::Right; 3];
    let mut why = Vec::new();
    for i in 0..3 {
        match here.ops[i] {
            Err(outcome) => verdicts[i] = outcome,
            Ok((v, bound)) => {
                if let Some(expected) = expected
                    && (v - expected[i]).abs() > slack(bound + here.a.1 + here.b.1)
                {
                    verdicts[i] = Outcome::Wrong;
                    why.push(format!(
                        "{} {v:e} for the closed form {:e}",
                        names[i], expected[i]
                    ));
                }
                if let Ok((w, wbound)) = world.ops[i]
                    && (v - w).abs() > slack(bound + wbound)
                {
                    verdicts[i] = Outcome::Wrong;
                    why.push(format!("{} {v:e}, in the world {w:e}", names[i]));
                }
            }
        }
    }
    // The identities, where both of their booleans were built and
    // neither is known wrong another way: which of the two is wrong they
    // do not say, so both are counted.
    let both = here.a.1 + here.b.1;
    if let (Ok((u, bu)), Ok((i, bi))) = (here.ops[0], here.ops[1])
        && (u + i - here.a.0 - here.b.0).abs() > slack(bu + bi + both)
        && verdicts[0] == Outcome::Right
        && verdicts[1] == Outcome::Right
    {
        verdicts[0] = Outcome::Wrong;
        verdicts[1] = Outcome::Wrong;
        why.push(format!(
            "union {u:e} + intersection {i:e} is not {:e}",
            here.a.0 + here.b.0
        ));
    }
    if let (Ok((s, bs)), Ok((i, bi))) = (here.ops[2], here.ops[1])
        && (s + i - here.a.0).abs() > slack(bs + bi + here.a.1)
        && verdicts[2] == Outcome::Right
        && verdicts[1] == Outcome::Right
    {
        verdicts[2] = Outcome::Wrong;
        verdicts[1] = Outcome::Wrong;
        why.push(format!(
            "subtraction {s:e} + intersection {i:e} is not {:e}",
            here.a.0
        ));
    }
    Some(Judged {
        what,
        frame,
        verdicts,
        why,
        refusals: here.refusals,
    })
}

/// Seeded pairs at `scales`, each `far` from the origin ([`judged`]).
fn booleans(seeds: std::ops::Range<u64>, scales: &[f64], far: f64) -> Booleans {
    let mut found = Booleans::default();
    for seed in seeds {
        for which in 0..PAIRS {
            for k in scales {
                let Some(judged) = judged(seed, which, *k, far) else {
                    found.not_made += 1;
                    continue;
                };
                for why in &judged.refusals {
                    *found.refusals.entry(why.clone()).or_default() += 1;
                }
                let label = format!("{} at {k:e}", PAIR_NAMES[which as usize]);
                let counts = found.counts.entry(label).or_default();
                for verdict in judged.verdicts {
                    counts[verdict as usize] += 1;
                }
                if !judged.why.is_empty() || judged.verdicts.contains(&Outcome::Invalid) {
                    found.wrong.push(format!(
                        "seed {seed} pair {which} ({}) at scale {k:e}, frame {:?}: {:?} {}",
                        judged.what,
                        judged.frame,
                        judged.verdicts,
                        judged.why.join("; ")
                    ));
                }
            }
        }
    }
    found
}

/// The smallest of them, in round numbers: a block 30 by 50 by 50 (times
/// 1e6, tolerance 1) whose side at y = 50 is a strip ruled across it (`u`
/// the 30 along x, `v` from 0 to 1 across the 50 along z), and a block 16
/// by 40 by 16 that stands in it and out through that side by 10. The
/// volumes of the union, the intersection and the subtraction, in the
/// world and in a turned frame.
fn through_a_ruled_side(frame: &Frame) -> [Result<f64, String>; 3] {
    let k = 1e6;
    let tol = tolerance(k);
    let mut a = primitives::block(frame, 30.0 * k, 50.0 * k, 50.0 * k).unwrap();
    let index = a
        .faces
        .iter()
        .position(|f| match &f.surface {
            Surface::Plane(p) => {
                p.frame().z().dot(frame.y()).abs() > 0.999_999
                    && (frame.local(p.frame().origin()).1 - 50.0 * k).abs() < 1.0
            }
            _ => false,
        })
        .unwrap();
    let corner = |x: f64, z: f64| frame.point(x * 30.0 * k, 50.0 * k, z * 50.0 * k);
    let strip = NurbsSurface::new(
        1,
        1,
        vec![0.0, 0.0, 30.0 * k, 30.0 * k],
        vec![0.0, 0.0, 1.0, 1.0],
        2,
        2,
        vec![
            corner(0.0, 0.0),
            corner(0.0, 1.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
        ],
        vec![1.0; 4],
    )
    .unwrap();
    // The strip's normal is against the frame's y: the side looks along it.
    a.faces[index].surface = Surface::Nurbs(strip);
    a.faces[index].reversed = true;
    assert_eq!(check(&a, &tol), []);
    let at = frame.at(frame.point(10.0 * k, 20.0 * k, 30.0 * k)).unwrap();
    let b = primitives::block(&at, 16.0 * k, 40.0 * k, 16.0 * k).unwrap();
    [BooleanOp::Union, BooleanOp::Intersect, BooleanOp::Subtract].map(|op| {
        let result = boolean(&a, &b, op, &tol).map_err(|e| shortened(&e.to_string()))?;
        if !check(&result.body, &tol).is_empty() {
            return Err("fails the checker".to_string());
        }
        mass_bound(&result.body)
            .map(|(m, _)| m.volume)
            .map_err(|e| format!("not measured: {e}"))
    })
}

/// The union of the two is both less what they share (30 · 50 · 50 + 16
/// · 40 · 16 − 16 · 30 · 16 = 77 560), the intersection what they share
/// (7 680), the subtraction the first less that (67 320), all times 1e18.
/// Before the repair the second block's edges were not found to cross
/// the ruled side, and all three were refused in the world's frame
/// ("boolean produced an invalid body: edge … is used … times"); with
/// other sizes the same pair came out wrong (the seeds below).
#[test]
fn a_block_through_a_ruled_side_of_a_block() {
    let turned = Frame::new(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(-0.68, -0.16, 0.71),
        Vec3::new(-0.05, 0.98, 0.17),
    )
    .unwrap();
    for (name, frame) in [("the world", Frame::WORLD), ("a turned frame", turned)] {
        let found = through_a_ruled_side(&frame);
        println!("{name}: {found:?}");
        for (volume, expected) in found.iter().zip([77_560.0, 7_680.0, 67_320.0]) {
            let volume = volume.clone().unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                (volume / 1e18 - expected).abs() < 1e-6 * expected,
                "{name}: {volume:e} for {expected} e18: {found:?}"
            );
        }
    }
}

/// The booleans that were built wrong (2026-11-14, seeds 4 and 8 of the
/// wide measure, at scale 1e6): a block with ruled sides and a block that
/// shares a corner of it, in a turned frame at the origin and 2.6e8 from
/// it. Of the first pair the union came out 12.5 % and the subtraction
/// 15 % over their closed forms (1.2418e23 for 1.1041e23, 1.0526e23 for
/// 9.149e22), of the second the union 1.8 % under (8.645e22 for
/// 8.805e22): solids that pass the checker and are not the result, where
/// an edge of one block was not found to cross a ruled side of the other
/// (the search for the nearest point of the side ended short,
/// `boolean/freeform.rs` › `descend`). The first union was both blocks
/// whole and the subtraction the first block untouched, as if the two
/// did not meet; the other union was the first block alone, the part of
/// the second that stands out of it gone. All three booleans of both
/// pairs are their closed forms.
#[test]
fn the_booleans_that_were_built_wrong_are_their_closed_forms() {
    for (seed, far) in [(4, 0.0), (4, 260.0), (8, 0.0), (8, 260.0)] {
        let judged = judged(seed, 1, 1e6, far).unwrap();
        assert_eq!(
            judged.verdicts,
            [Outcome::Right; 3],
            "seed {seed}, {far} from the origin: {}: {:?} {:?}",
            judged.what,
            judged.why,
            judged.refusals
        );
    }
}

/// No boolean of the seeded pairs is built wrong, at any scale, in
/// turned frames at the distance of the finding (some 250 model sizes at
/// scale 1e6 are 2.6e8 units) and at the origin.
#[test]
fn the_booleans_of_solids_with_ruled_sides_are_their_volumes() {
    for far in [0.0, 260.0] {
        let found = booleans(0..1, &SCALES, far);
        let name = format!("booleans {far} from the origin");
        println!("{}", found.table(&name));
        assert_eq!(found.wrong(), 0, "\n{}", found.table(&name));
    }
}

/// The wide measure, by hand: `SWEEP_SEEDS` seeds (default 12) from
/// `SWEEP_FROM`, at scale 1e6 where the ruled sides' tangents differ by
/// millions, and at every scale.
#[test]
#[ignore = "a wide measure, run by hand"]
fn the_booleans_of_solids_with_ruled_sides_wide() {
    let mut wrong = 0;
    let mut tables = String::new();
    for (scales, far) in [(&[1e6][..], 260.0), (&[1e6][..], 0.0), (&SCALES[..], 5e6)] {
        let found = booleans(seeds_asked(12), scales, far);
        let name = format!("booleans (wide) at {scales:?}, {far} from the origin");
        tables += &found.table(&name);
        wrong += found.wrong();
    }
    println!("{tables}");
    assert_eq!(wrong, 0, "\n{tables}");
}

// ---------------------------------------------------------------------
// What the repair still ends short on: owed (the review of 2026-11-15)
// ---------------------------------------------------------------------

/// The circle of radius 1 as a rational quadratic B-spline: nine control
/// points `(x, y, weight)` on [`RING_KNOTS`].
const RING: [(f64, f64, f64); 9] = [
    (1.0, 0.0, 1.0),
    (1.0, 1.0, std::f64::consts::FRAC_1_SQRT_2),
    (0.0, 1.0, 1.0),
    (-1.0, 1.0, std::f64::consts::FRAC_1_SQRT_2),
    (-1.0, 0.0, 1.0),
    (-1.0, -1.0, std::f64::consts::FRAC_1_SQRT_2),
    (0.0, -1.0, 1.0),
    (1.0, -1.0, std::f64::consts::FRAC_1_SQRT_2),
    (1.0, 0.0, 1.0),
];
const RING_KNOTS: [f64; 12] = [
    0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
];

/// A cone of radius `r` and height `h` on the xy plane of `frame`, its
/// side the exact rational NURBS of it (`u` around, `v` up the slant on
/// knots times `pv`), and the analytic cone it stands for.
fn nurbs_cone(frame: &Frame, r: f64, h: f64, pv: f64) -> (Body, Body) {
    let twin = primitives::cone(frame, r, h).unwrap();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (x, y, weight) in RING {
        for (rho, z) in [(r, 0.0), (0.0, h)] {
            ctrl.push(frame.point(rho * x, rho * y, z));
            weights.push(weight);
        }
    }
    let side = Surface::Nurbs(
        NurbsSurface::new(
            2,
            1,
            RING_KNOTS.to_vec(),
            vec![0.0, 0.0, pv, pv],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap(),
    );
    let mut body = twin.clone();
    for face in &mut body.faces {
        if face.surface.kind() == "cone" {
            face.surface = side.clone();
        }
    }
    (body, twin)
}

/// A boolean of a cone and a block: the frame, the cone (radius,
/// height), the block (its levels as parts of the height) and the
/// operation.
type ConeCase = (&'static str, f64, f64, (f64, f64), BooleanOp);

/// The booleans of a cone whose side is on knots of `v` stretched by 3e7
/// whose volume is not measured today.
const CONE_NOT_MEASURED: [ConeCase; 6] = [
    ("world", 10.561, 37.486, (0.6, 1.4), BooleanOp::Intersect),
    ("world", 10.561, 37.486, (0.2, 0.55), BooleanOp::Union),
    ("world", 10.561, 37.486, (0.2, 0.55), BooleanOp::Subtract),
    ("far", 10.0, 37.5, (0.6, 1.4), BooleanOp::Intersect),
    ("far", 10.0, 37.5, (0.2, 0.55), BooleanOp::Union),
    ("far", 10.0, 37.5, (0.2, 0.55), BooleanOp::Subtract),
];

/// **Owed** (the review of 2026-11-15): booleans of a cone whose side is
/// the exact NURBS of it on knots of `v` stretched 3e7 times, with a
/// block over its tip or through its middle, build a body that passes
/// the checker and has as many faces as the analytic cone's boolean, but
/// whose volume cannot be measured: the mass integration says a face "is
/// not bounded by its loops", and the tessellator would need some 13 000
/// segments a side of a patch at a chord of 1e-3 (its limit is 1024), so
/// neither tells whether the body is right. The other booleans of the
/// same pairs, and all of them on even knots, are their closed forms. 12
/// of the review's 972 booleans were so (cones on knots of `v` times 3e7,
/// or at unit speed at scale 1e6), refused on the base too. When
/// one of [`CONE_NOT_MEASURED`] is measured (or refused), this test fails:
/// hold it to its closed form and take it off the list.
#[test]
fn booleans_of_a_cone_on_stretched_knots_whose_volume_is_not_measured_owed() {
    use std::f64::consts::PI;
    let tol = tolerance(1.0);
    let far = Frame::new(
        Vec3::new(0.61, -0.52, 0.6) * 260.0,
        Vec3::new(-0.5, 0.2, 0.84),
        Vec3::new(0.2, 1.0, 0.1),
    )
    .unwrap();
    let mut owed = 0;
    for (name, frame, r, h) in [
        ("world", Frame::WORLD, 10.561, 37.486),
        ("far", far, 10.0, 37.5),
    ] {
        for pv in [1.0, 3e7] {
            let (cone, twin) = nurbs_cone(&frame, r, h, pv);
            assert!(check(&cone, &tol).is_empty());
            for (low, high) in [(0.6, 1.4), (0.2, 0.55)] {
                let (z0, z1) = (low * h, high * h);
                let at = frame.at(frame.point(-2.0 * r, -2.0 * r, z0)).unwrap();
                let block = primitives::block(&at, 4.0 * r, 4.0 * r, z1 - z0).unwrap();
                let rho = |z: f64| r * (1.0 - z.min(h) / h);
                let (r0, r1) = (rho(z0), rho(z1));
                let shared = PI / 3.0 * (z1.min(h) - z0) * (r0 * r0 + r0 * r1 + r1 * r1);
                let (va, vb) = (PI / 3.0 * r * r * h, 16.0 * r * r * (z1 - z0));
                for (op, closed) in [
                    (BooleanOp::Union, va + vb - shared),
                    (BooleanOp::Intersect, shared),
                    (BooleanOp::Subtract, va - shared),
                ] {
                    let what = format!(
                        "{op:?} of the cone in {name} on knots of v times {pv:e} and the block {low} to {high}"
                    );
                    let result =
                        boolean(&cone, &block, op, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let analytic = boolean(&twin, &block, op, &tol).unwrap();
                    assert!(check(&result.body, &tol).is_empty(), "{what}");
                    assert_eq!(result.body.faces.len(), analytic.body.faces.len(), "{what}");
                    let listed = pv > 1.0
                        && CONE_NOT_MEASURED
                            .iter()
                            .any(|c| (c.0, c.1, c.3, c.4) == (name, r, (low, high), op));
                    match mass_bound(&result.body) {
                        Err(e) if listed => {
                            assert!(
                                e.to_string().contains("is not bounded by its loops"),
                                "{what}: {e}"
                            );
                            owed += 1;
                        }
                        Err(e) => panic!("{what}: {e}"),
                        Ok((mass, _)) if listed => panic!(
                            "{what} is measured now ({:e} for the closed form {closed:e}): hold it to that and take it off the list",
                            mass.volume
                        ),
                        Ok((mass, bound)) => assert!(
                            (mass.volume - closed).abs() <= 1e-8 * (va + vb) + 4.0 * bound.volume,
                            "{what}: {:e} for {closed:e}",
                            mass.volume
                        ),
                    }
                }
            }
        }
    }
    assert_eq!(owed, CONE_NOT_MEASURED.len());
}

/// A block of `size` in `frame` whose bottom is a ruled NURBS face whose
/// speed along `u` rises from `2 slow` of its width to twice it (a
/// quadratic whose middle control point is `slow` of the way along), or
/// the bilinear face of the plane at even speed (`slow` 0).
fn block_with_a_ruled_bottom(frame: &Frame, size: Vec3, slow: f64) -> Body {
    let mut body = primitives::block(frame, size.x, size.y, size.z).unwrap();
    let bottom = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                let (_, _, z) = frame.local(plane.frame().origin());
                plane.frame().z().dot(frame.z()).abs() > 0.99 && z.abs() <= 1e-9 * size.length()
            }
            _ => false,
        })
        .unwrap();
    let Surface::Plane(plane) = &body.faces[bottom].surface else {
        panic!("the bottom is not a plane")
    };
    let f = *plane.frame();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for vertex in &body.vertices {
        let (x, y, z) = f.local(vertex.point);
        if z.abs() <= 1e-9 * size.length() {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    let at = |x: f64, y: f64| {
        f.point(
            lo[0] + x * (hi[0] - lo[0]),
            lo[1] + y * (hi[1] - lo[1]),
            0.0,
        )
    };
    let line = vec![0.0, 0.0, 1.0, 1.0];
    let surface = if slow > 0.0 {
        NurbsSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            line,
            3,
            2,
            vec![
                at(0.0, 0.0),
                at(0.0, 1.0),
                at(slow, 0.0),
                at(slow, 1.0),
                at(1.0, 0.0),
                at(1.0, 1.0),
            ],
            vec![1.0; 6],
        )
    } else {
        NurbsSurface::new(
            1,
            1,
            line.clone(),
            line,
            2,
            2,
            vec![at(0.0, 0.0), at(0.0, 1.0), at(1.0, 0.0), at(1.0, 1.0)],
            vec![1.0; 4],
        )
    };
    body.faces[bottom].surface = Surface::Nurbs(surface.unwrap());
    body
}

/// **Owed, a cost** (the review of 2026-11-15): a point three tolerances
/// below a block whose bottom is a ruled face whose speed along `u` rises
/// from 2e-4 of its width to twice it is placed outside, as it is, but
/// costs some 18.6 million units of work (`craft3d_geom::allowance`; 3 s
/// on a laptop), where beside the same bottom at even speed it costs a
/// few ten thousand. From 2e-6 of the width (the review's slabs) a point
/// costs 1.6 to 2.6 s on the build server, many are left unknown (4 of
/// 8), and a boolean with such a block was refused or not done in 150 s.
/// The count is the work's, not the clock's, and the same on every
/// machine. When the point costs less than a million units, this test
/// fails: the cost is repaired, take it off.
#[test]
fn a_point_beside_a_ruled_face_whose_speed_rises_ten_thousandfold_costs_millions_owed() {
    let tol = tolerance(1.0);
    let size = Vec3::new(30.0, 20.0, 12.0);
    let p = Vec3::new(15.0, 10.0, -3e-6);
    for (slow, owed) in [(0.0, false), (1e-4, true)] {
        let body = block_with_a_ruled_bottom(&Frame::WORLD, size, slow);
        assert!(check(&body, &tol).is_empty(), "{slow}");
        let (places, units) =
            craft3d_geom::allowance::measured(|| classify_points(&body, &[p], &tol));
        assert_eq!(places.unwrap(), vec![Place::Outside], "{slow}");
        if owed {
            assert!(
                units > 5_000_000,
                "a point beside the bottom rising from {slow} of its width costs {units} units now: the cost is repaired, take this test off"
            );
        } else {
            assert!(units < 1_000_000, "at even speed: {units}");
        }
    }
}

/// A block of `size` in the world's frame whose faces facing +z, +x and
/// +y are flat bicubic NURBS of six by six control points whose weights
/// run from 1e-3 to 1e3 (seeded by `seed`): the planes, drawn at speeds
/// that change a millionfold over them.
fn block_with_weighted_sides(size: Vec3, seed: u64) -> Body {
    let mut body = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
    let mut state = seed;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    for index in 0..body.faces.len() {
        let Surface::Plane(plane) = &body.faces[index].surface else {
            continue;
        };
        let f = *plane.frame();
        if !(f.z().x > 0.99 || f.z().y > 0.99 || f.z().z > 0.99) {
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for vertex in &body.vertices {
            let (x, y, z) = f.local(vertex.point);
            if z.abs() <= 1e-9 * size.length() {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                ctrl.push(f.point(
                    lo[0] + (hi[0] - lo[0]) * i as f64 / 5.0,
                    lo[1] + (hi[1] - lo[1]) * j as f64 / 5.0,
                    0.0,
                ));
                weights.push((6.0 * (next() - 0.5) * std::f64::consts::LN_10).pexp());
            }
        }
        body.faces[index].surface = Surface::Nurbs(
            NurbsSurface::new(3, 3, knots.clone(), knots.clone(), 6, 6, ctrl, weights).unwrap(),
        );
    }
    body
}

/// **Owed** (the review of 2026-11-15): a block 30 by 900 by 20 whose
/// sides facing +x, +y and +z are the planes drawn as weighted NURBS
/// (weights from 1e-3 to 1e3) is valid, but the checker reads an edge of
/// it as 1.65 from the face it bounds, so a boolean with it is refused
/// ("the first operand of a boolean produced an invalid body"): the edge
/// is a side of the face's rectangle, which the face's border runs along
/// (its control points lie on it), and the projection the checker asks
/// (`Surface::project`, not the boolean's patch tree) ends short on such
/// a face, as the boolean's nearest point did on a face whose tangents
/// differ millions of times. The block 30 by 30 passes. Booleans of
/// blocks whose weighted sides pass the checker (30 by 30 and 30 by 90,
/// eight seeds, a block over a corner) were built with the faces and the
/// vertices of the plain blocks' booleans or refused ("a freeform face
/// whose parametrisation stalls or folds"), none wrong as far as can be
/// told; their volumes are not measured (the mass integration takes
/// weights at most 1e4 apart within a span). When the long block passes
/// the checker, this test fails: hold its booleans to their closed forms.
#[test]
fn a_block_whose_long_side_is_a_weighted_sheet_is_read_as_invalid_owed() {
    let tol = tolerance(1.0);
    let square = block_with_weighted_sides(Vec3::new(30.0, 30.0, 20.0), 1);
    assert!(check(&square, &tol).is_empty());
    let long = block_with_weighted_sides(Vec3::new(30.0, 900.0, 20.0), 1);
    let defects = check(&long, &tol);
    assert!(
        defects
            .iter()
            .any(|d| d.to_string().contains("away from the surface of face")),
        "the long block passes the checker now: {defects:?}; hold its booleans to their closed forms"
    );
    let at = Frame::WORLD.at(Vec3::new(12.0, 315.0, 9.0)).unwrap();
    let tool = primitives::block(&at, 27.0, 765.0, 16.0).unwrap();
    let refused = boolean(&long, &tool, BooleanOp::Intersect, &tol);
    assert!(
        refused.is_err(),
        "{:?}",
        refused.map(|r| r.body.faces.len())
    );
}
