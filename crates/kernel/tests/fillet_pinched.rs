//! A fillet whose section runs to nothing at a place inside its edge is
//! refused, with the place (docs/design/blends.md › *A fillet pinched
//! inside its edge*). What 0.1.2 returned there was a valid solid whose
//! fillet face had no width at that place and, between two such places,
//! its two borders crossed: a body that could not be measured.
//!
//! **The shapes** are lofts through "house" sections (a rectangle 20 wide
//! and 8 high with a ridge `a` above its top's middle), the sections one
//! after the other along the frame's z: the two roof faces meet in the
//! ridge edge. Every section's roof is two straight lines, so across the
//! ridge each roof face is straight from the ridge down to its eave, which
//! is level: at the station `x` along the loft a roof face is
//! `height = H + a(x)·(1 − |y| / W)`, with `a(x)` the height of the ridge
//! edge itself above the eaves. The oracles below take `a` and its slope
//! `a′` from that edge of the lofted body (a straight line in a ruled loft,
//! a spline in a smooth one) and nothing else from the kernel:
//!
//! - **the turn across the ridge**: the roof's upward normals at the ridge
//!   are `(−a′, ±a/W, 1)` (along, across, up), so the two faces turn
//!   against each other by φ with `tan(φ/2) = (a/W) / √(1 + a′²)`: zero
//!   where the ridge comes down to the eaves, convex where it is above
//!   them and concave where it is below;
//! - **a chamfer's section** (`chamfer_removed`): the two cut lines mirror
//!   each other, so the chamfer face is level across and the section in a
//!   plane x = const is a triangle of area `a·y²/W`, `y` the half width of
//!   the chamfer there. In a ruled loft (`a` linear, slope k) `y = d /
//!   √(1 + (a/W)²/(1 + k²))` and the volume is in closed form;
//! - **a fillet's section** (`fillet_removed`): the ball that touches both
//!   roof faces has its centre in the plane of symmetry, and touches each
//!   face where the centre is the radius below it along its normal; the
//!   fillet face is the arc of that ball between the two feet, and the
//!   section removed in a plane x = const is integrated between the roof
//!   and those arcs (Gauss-Legendre in both directions).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, mass, tolerance};
use common::Rng;
use craft3d_geom::Curve3;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::fillet::{FilletEdge, FilletFace, FilletResult, fillet};
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::{FilletPinch, KernelError};
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check, mass_properties};

/// Half the width of the house and the height of its eaves.
const W: f64 = 10.0;
const H: f64 = 8.0;

/// The world and two frames away from the origin, one of them tilted.
fn frames(k: f64) -> Vec<Frame> {
    let mut all = common::frames(k);
    all.push(
        Frame::new(
            Vec3::new(400.0 * k, -300.0 * k, 250.0 * k),
            Vec3::new(-2.0, 1.0, 2.0),
            Vec3::new(0.0, 1.0, -0.5),
        )
        .unwrap(),
    );
    all
}

/// The loft through house sections `step` apart along the frame's z, the
/// ridge `heights[i]` above the eaves at section `i`.
fn ridge(frame: &Frame, k: f64, heights: &[f64], step: f64, ruled: bool) -> Body {
    let tol = tolerance(k);
    let sections: Vec<Section> = heights
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let points: Vec<[f64; 2]> = [[-W, 0.0], [W, 0.0], [W, H], [0.0, H + a], [-W, H]]
                .iter()
                .map(|p| [p[0] * k, p[1] * k])
                .collect();
            let curves = common::polygon(1, 1, &points);
            let profiles = detect_profiles(&curves, &tol).unwrap();
            Section {
                plane: frame
                    .at(frame.point(0.0, 0.0, i as f64 * step * k))
                    .unwrap(),
                outline: profiles[0].outer.clone(),
                start: None,
            }
        })
        .collect();
    let options = Options {
        ruled,
        closed: false,
    };
    let body = loft(&sections, &options, &tol).unwrap().body;
    assert_eq!(check(&body, &tol), []);
    body
}

/// The edges of the ridge, from the first section on: those that run
/// along the loft in the plane of symmetry, above the base.
fn ridge_edges(body: &Body, frame: &Frame, k: f64) -> Vec<EdgeId> {
    let mut found: Vec<(f64, EdgeId)> = Vec::new();
    for (i, e) in body.edges.iter().enumerate() {
        let on = |t: f64| {
            let (x, y, _) = frame.local(e.curve.eval(t));
            x.abs() <= 1e-6 * k && y > 0.2 * H * k
        };
        let (a, b) = e.range;
        let (_, _, z0) = frame.local(e.curve.eval(a));
        let (_, _, z1) = frame.local(e.curve.eval(b));
        if on(a) && on(b) && on(0.5 * (a + b)) && (z1 - z0).abs() > 1e-3 * k {
            found.push((z0.min(z1), EdgeId(i as u32)));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|f| f.1).collect()
}

fn first_face(body: &Body, edge: EdgeId) -> FaceId {
    let i = (0..body.faces.len())
        .find(|i| {
            body.faces[*i]
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .unwrap();
    FaceId(i as u32)
}

fn round(body: &Body, edge: EdgeId, radius: f64, k: f64) -> Result<FilletResult, KernelError> {
    fillet(body, &[FilletEdge { edge, radius }], &tolerance(k))
}

fn bevel(body: &Body, edge: EdgeId, distance: f64, k: f64) -> Result<Body, KernelError> {
    let size = ChamferSize::Distances {
        first: distance,
        second: distance,
    };
    let edges = [ChamferEdge {
        edge,
        first: first_face(body, edge),
        size,
    }];
    chamfer(body, &edges, &tolerance(k)).map(|made| made.body)
}

/// The ridge edge of a lofted roof, as the oracles read it: along (the
/// frame's z), up (its y), in the loft's own lengths.
struct Roof {
    frame: Frame,
    curve: Curve3,
    /// The curve's parameters at the edge's first and last station.
    range: (f64, f64),
    /// The length of the loft along the frame's z.
    length: f64,
    k: f64,
}

impl Roof {
    fn of(body: &Body, edge: EdgeId, frame: &Frame, k: f64) -> Roof {
        let e = body.edge(edge).unwrap();
        let along = |t: f64| frame.local(e.curve.eval(t)).2;
        let (lo, hi) = (along(e.range.0), along(e.range.1));
        let range = if lo <= hi {
            e.range
        } else {
            (e.range.1, e.range.0)
        };
        Roof {
            frame: *frame,
            curve: e.curve.clone(),
            range,
            length: (hi - lo).abs(),
            k,
        }
    }

    /// The station of a point: how far along the loft it is from the
    /// edge's first end.
    fn station(&self, p: Vec3) -> f64 {
        self.frame.local(p).2 - self.frame.local(self.curve.eval(self.range.0)).2
    }

    /// The curve's parameter at the station `x`, also past the ends, where
    /// the curve goes on as it is.
    fn parameter(&self, x: f64) -> f64 {
        let (t0, t1) = self.range;
        let mut t = t0 + (t1 - t0) * x / self.length;
        for _ in 0..60 {
            let (p, d) = self.curve.derivatives(t);
            let step = (self.station(p) - x) / d.dot(self.frame.z());
            t -= step;
            if step.abs() <= 1e-15 * (t1 - t0).abs() {
                break;
            }
        }
        t
    }

    /// The ridge's height above the eaves at the station `x`, and its
    /// slope there.
    fn height(&self, x: f64) -> (f64, f64) {
        let (p, d) = self.curve.derivatives(self.parameter(x));
        let a = self.frame.local(p).1 - H * self.k;
        (a, d.dot(self.frame.y()) / d.dot(self.frame.z()))
    }

    /// The tangent of half the turn across the ridge at `x`: positive
    /// where it is convex.
    fn half_tan(&self, x: f64) -> f64 {
        let (a, slope) = self.height(x);
        a / (W * self.k) / (1.0 + slope * slope).sqrt()
    }

    /// Where the height passes through zero between the stations `lo` and
    /// `hi`, on which it has different signs.
    fn zero(&self, lo: f64, hi: f64) -> f64 {
        let (mut lo, mut hi) = (lo, hi);
        let rising = self.height(hi).0 > 0.0;
        for _ in 0..200 {
            let middle = 0.5 * (lo + hi);
            if (self.height(middle).0 > 0.0) == rising {
                hi = middle;
            } else {
                lo = middle;
            }
        }
        0.5 * (lo + hi)
    }

    /// Where the height is least between `lo` and `hi`.
    fn lowest(&self, lo: f64, hi: f64) -> f64 {
        let (mut lo, mut hi) = (lo, hi);
        for _ in 0..300 {
            let (c, d) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if self.height(c).0 < self.height(d).0 {
                hi = d;
            } else {
                lo = c;
            }
        }
        0.5 * (lo + hi)
    }

    /// The point of the ridge at the station `x`.
    fn point(&self, x: f64) -> Vec3 {
        self.curve.eval(self.parameter(x))
    }
}

/// The nodes and weights of the five-point Gauss-Legendre rule on (0, 1).
fn gauss5() -> [(f64, f64); 5] {
    let a = (5.0 - 2.0 * (10.0_f64 / 7.0).sqrt()).sqrt() / 3.0;
    let b = (5.0 + 2.0 * (10.0_f64 / 7.0).sqrt()).sqrt() / 3.0;
    let wa = (322.0 + 13.0 * 70.0_f64.sqrt()) / 900.0;
    let wb = (322.0 - 13.0 * 70.0_f64.sqrt()) / 900.0;
    [
        (0.5 - 0.5 * b, 0.5 * wb),
        (0.5 - 0.5 * a, 0.5 * wa),
        (0.5, 0.5 * 128.0 / 225.0),
        (0.5 + 0.5 * a, 0.5 * wa),
        (0.5 + 0.5 * b, 0.5 * wb),
    ]
}

/// The integral of `f` from `lo` to `hi`, in `panels` panels of five
/// Gauss-Legendre points.
fn integral(lo: f64, hi: f64, panels: usize, f: impl Fn(f64) -> f64) -> f64 {
    let width = (hi - lo) / panels as f64;
    let mut sum = 0.0;
    for i in 0..panels {
        for (node, weight) in gauss5() {
            sum += weight * width * f(lo + width * (i as f64 + node));
        }
    }
    sum
}

/// The volume a chamfer of the ridge by the distance `d` on both faces
/// takes from the roof (less what it adds where the ridge is a valley):
/// see the module documentation. The half width `y` of the chamfer at the
/// station `x` belongs to the plane across the ridge at the station `s`
/// that passes through the cut line's point there; `s` and `y` are found
/// together (a fixed point: the half width from the distance in that
/// plane, the plane's station from the half width).
fn chamfer_removed(roof: &Roof, d: f64) -> f64 {
    let w = W * roof.k;
    integral(0.0, roof.length, 400, |x| {
        let (a, _) = roof.height(x);
        let mut s = x;
        let mut y = d;
        for _ in 0..200 {
            let (at_s, slope) = roof.height(s);
            // (x − s)² + y² + (a (1 − y/w) − a(s))² = d², the larger root.
            let (p, q) = (a / w, a - at_s);
            let (qa, qb, qc) = (1.0 + p * p, -2.0 * p * q, (x - s) * (x - s) + q * q - d * d);
            y = (-qb + (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
            // The cut line's point lies in the plane across the ridge at s.
            let next = x + slope * (a * (1.0 - y / w) - at_s);
            let moved = (next - s).abs();
            s = next;
            if moved <= 1e-15 * roof.length {
                break;
            }
        }
        a * y * y / w
    })
}

/// [`chamfer_removed`] for a ruled loft from the height `a0` to `a1` over
/// `length`, in closed form.
fn chamfer_removed_ruled(a0: f64, a1: f64, length: f64, d: f64, k: f64) -> f64 {
    let w = W * k;
    let slope = (a1 - a0) / length;
    let lift = 1.0 + slope * slope;
    let log = |a: f64| (1.0 + (a / w) * (a / w) / lift).pln();
    w * d * d * lift / (2.0 * slope) * (log(a1) - log(a0))
}

/// The ball of `radius` that touches both roof faces with its feet at the
/// station `x`: its centre (along, up), the direction from the centre to
/// the middle of its arc (along, up), and the half angle of the arc.
fn ball(roof: &Roof, x: f64, radius: f64) -> ([f64; 2], [f64; 2], f64) {
    let w = W * roof.k;
    let (a, slope) = roof.height(x);
    // The roof's unit normal (along, across, up) at the distance y from
    // the ridge; the foot is where the centre, the radius below it along
    // the normal, lies in the plane of symmetry.
    let normal = |y: f64| {
        let n = [-slope * (1.0 - y / w), a / w, 1.0];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        [n[0] / length, n[1] / length, n[2] / length]
    };
    let mut y = 0.0;
    for _ in 0..200 {
        let next = radius * normal(y)[1];
        let moved = (next - y).abs();
        y = next;
        if moved <= 1e-16 * radius {
            break;
        }
    }
    let n = normal(y);
    let foot_up = H * roof.k + a * (1.0 - y / w);
    let centre = [x - radius * n[0], foot_up - radius * n[2]];
    let cosine = (n[0] * n[0] + n[2] * n[2]).sqrt();
    (centre, [n[0] / cosine, n[2] / cosine], n[1].patan2(cosine))
}

/// The volume a fillet of `radius` takes from the ridge of the roof: see
/// the module documentation. In the plane at the station `x` the fillet
/// face is cut in the points of the arcs that lie in it: per angle θ
/// along the arc (the point is `radius·sin θ` from the plane of symmetry)
/// the ball whose arc passes through the plane there is found, and the
/// height between the roof and that point is integrated across.
fn fillet_removed(roof: &Roof, radius: f64) -> f64 {
    let w = W * roof.k;
    integral(0.0, roof.length, 400, |x| {
        let (a, _) = roof.height(x);
        let (_, _, reach) = ball(roof, x, radius);
        2.0 * integral(0.0, reach, 6, |theta| {
            let (sine, cosine) = theta.psin_cos();
            // The ball whose arc is at the station x at this angle.
            let mut foot = x;
            let mut found = ball(roof, foot, radius);
            for _ in 0..200 {
                let (centre, middle, _) = found;
                let off = centre[0] + radius * cosine * middle[0] - x;
                foot -= off;
                found = ball(roof, foot, radius);
                if off.abs() <= 1e-15 * roof.length {
                    break;
                }
            }
            let (centre, middle, _) = found;
            let on_roof = H * roof.k + a * (1.0 - radius * sine / w);
            let on_arc = centre[1] + radius * cosine * middle[1];
            (on_roof - on_arc) * radius * cosine
        })
    })
}

/// The fillet face of a built fillet between the stations of its edge:
/// the least distance between its two borders at one parameter along it,
/// and whether the borders keep to their own sides all the way (the
/// second border on the side of the frame's +x or of its −x throughout).
/// The stretches of the surface past the ends of the edge, which the end
/// faces cut off, are left out.
fn borders(made: &FilletResult, roof: &Roof) -> (f64, bool) {
    let index = made
        .faces
        .iter()
        .position(|f| matches!(f, FilletFace::Fillet(_)))
        .unwrap();
    let surface = &made.body.faces[index].surface;
    let ((u0, u1), (v0, v1)) = surface.domain();
    let inside = |p: Vec3| {
        let along = roof.station(p);
        along >= 0.0 && along <= roof.length
    };
    let n = 20000;
    let mut least = f64::INFINITY;
    let (mut one, mut other) = (false, false);
    for i in 0..=n {
        let u = u0 + (u1 - u0) * f64::from(i) / f64::from(n);
        let (p, q) = (surface.eval(u, v0), surface.eval(u, v1));
        if !(inside(p) && inside(q)) {
            continue;
        }
        let across = (q - p).dot(roof.frame.x());
        least = least.min(p.distance(q));
        one |= across > 0.0;
        other |= across < 0.0;
    }
    // A face with no station between the ends (a cylinder along a
    // straight edge between planes, whose parameters are not along the
    // edge) is not looked at.
    (least, one != other || least == f64::INFINITY)
}

/// How far the volume a blend takes may differ from its oracle's: the
/// blend face is fitted within a quarter of the tolerance of the exact
/// one, over an area of the edge's length times the face's width, `wide`
/// at most (a chamfer's two distances; a fillet's arc, which on these
/// roofs turns by less than 1.5 radians).
fn within(roof: &Roof, wide: f64) -> f64 {
    0.25 * tolerance(roof.k).linear * roof.length * wide
}

/// The place a refusal names and why, for a fillet refused as pinched.
fn pinched(result: Result<FilletResult, KernelError>, what: &str) -> (Vec3, FilletPinch, usize) {
    match result {
        Err(KernelError::FilletPinched {
            at, why, places, ..
        }) => (at, why, places),
        Err(other) => panic!("{what}: refused otherwise: {other}"),
        Ok(_) => panic!("{what}: built"),
    }
}

/// The four ridges of the finding, at three radii, three scales and in
/// three frames: each is refused with the place where its faces are
/// tangent, which the test finds on the ridge edge itself (where its
/// height above the eaves is zero). 0.1.2 returned a body for the first
/// (at the radii 0.3 and 3: valid, its fillet face pinched to nothing in
/// two places with its borders crossed between them, a body that could not
/// be measured), said the second and the third were "tangent along this
/// edge", and gave the fourth up as an internal error.
#[test]
fn the_ridges_whose_faces_become_tangent_inside_are_refused_with_the_place() {
    for k in SCALES {
        for frame in frames(k) {
            let near = |at: Vec3, want: Vec3, what: &str| {
                let off = at.distance(want);
                assert!(
                    off <= 1e-5 * k,
                    "{what} at scale {k}: the place is {off} off"
                );
            };
            // The ridge 6, 2, 0, 2: its smooth interpolant passes through
            // zero at the third section, dips below the eaves and comes
            // back.
            let body = ridge(&frame, k, &[6.0, 2.0, 0.0, 2.0], 15.0, false);
            let edge = ridge_edges(&body, &frame, k)[0];
            let roof = Roof::of(&body, edge, &frame, k);
            let bottom = roof.lowest(25.0 * k, 40.0 * k);
            assert!(roof.height(bottom).0 < -1e-3 * k, "the ridge dips");
            let down = roof.zero(20.0 * k, bottom);
            let up = roof.zero(bottom, 40.0 * k);
            assert!(
                (down - 30.0 * k).abs() <= 1e-6 * k,
                "tangent at the third section"
            );
            assert!(up - down > 2.0 * k, "and again {} further", up - down);
            for r in [0.3, 1.0, 3.0] {
                let (at, why, places) = pinched(round(&body, edge, r * k, k), "ridge 6 2 0 2");
                near(at, roof.point(down), "ridge 6 2 0 2");
                assert_eq!(why, FilletPinch::Flips);
                assert_eq!(places, 2);
            }
            // 6, 0, 6: tangent at the middle section, where the edge's
            // middle is, convex on both sides.
            let body = ridge(&frame, k, &[6.0, 0.0, 6.0], 25.0, false);
            let edge = ridge_edges(&body, &frame, k)[0];
            let roof = Roof::of(&body, edge, &frame, k);
            for r in [0.3, 1.0, 3.0] {
                let (at, why, places) = pinched(round(&body, edge, r * k, k), "ridge 6 0 6");
                near(at, roof.point(25.0 * k), "ridge 6 0 6");
                assert_eq!(why, FilletPinch::Tangent);
                assert_eq!(places, 1);
            }
            // A chamfer's side is read at the edge's middle: said so, with
            // the place, not as an edge tangent all along.
            match bevel(&body, edge, k, k) {
                Err(KernelError::ChamferTangentMiddle { at, .. }) => {
                    near(at, roof.point(25.0 * k), "the chamfer of ridge 6 0 6");
                }
                other => panic!("the chamfer of ridge 6 0 6: {other:?}"),
            }
            // 6, 0, −6: from convex to concave at the middle.
            let body = ridge(&frame, k, &[6.0, 0.0, -6.0], 25.0, false);
            let edge = ridge_edges(&body, &frame, k)[0];
            let roof = Roof::of(&body, edge, &frame, k);
            for r in [0.3, 1.0, 3.0] {
                let (at, why, places) = pinched(round(&body, edge, r * k, k), "ridge 6 0 -6");
                near(at, roof.point(25.0 * k), "ridge 6 0 -6");
                assert_eq!(why, FilletPinch::Flips);
                assert_eq!(places, 1);
            }
            assert!(matches!(
                bevel(&body, edge, k, k),
                Err(KernelError::ChamferTangentMiddle { .. })
            ));
            // 6, 3, −3: convex at the middle, concave near the far end.
            let body = ridge(&frame, k, &[6.0, 3.0, -3.0], 25.0, false);
            let edge = ridge_edges(&body, &frame, k)[0];
            let roof = Roof::of(&body, edge, &frame, k);
            let turns = roof.zero(25.0 * k, 50.0 * k);
            for r in [0.3, 1.0, 3.0] {
                let (at, why, places) = pinched(round(&body, edge, r * k, k), "ridge 6 3 -3");
                near(at, roof.point(turns), "ridge 6 3 -3");
                assert_eq!(why, FilletPinch::Flips);
                assert_eq!(places, 1);
            }
        }
    }
}

/// What the refusals say: the place, that no radius helps, and nothing
/// about an edge "tangent along" itself.
#[test]
fn the_refusals_name_the_place_and_say_what_is_true() {
    let (k, frame) = (1.0, Frame::WORLD);
    let body = ridge(&frame, k, &[6.0, 2.0, 0.0, 2.0], 15.0, false);
    let edge = ridge_edges(&body, &frame, k)[0];
    let said = round(&body, edge, 3.0, k).unwrap_err().to_string();
    assert!(
        said.contains("convex on one side of (0, 8, 30) and concave on the other"),
        "{said}"
    );
    assert!(said.contains("the first of 2 such places"), "{said}");
    assert!(said.contains("whatever the radius"), "{said}");
    assert!(!said.contains("tangent along"), "{said}");
    let body = ridge(&frame, k, &[6.0, 0.0, 6.0], 25.0, false);
    let edge = ridge_edges(&body, &frame, k)[0];
    let said = round(&body, edge, 3.0, k).unwrap_err().to_string();
    assert!(
        said.contains("become tangent at (0, 8, 25), a place inside the edge"),
        "{said}"
    );
    assert!(!said.contains("tangent along"), "{said}");
    let said = bevel(&body, edge, 1.0, k).unwrap_err().to_string();
    assert!(
        said.contains("tangent at the middle of the edge, at (0, 8, 25), though not all along it"),
        "{said}"
    );
    // An edge that is tangent all along is still said to be.
    let flat = ridge(&frame, k, &[0.0, 0.0], 40.0, true);
    let edge = ridge_edges(&flat, &frame, k)[0];
    let said = round(&flat, edge, 1.0, k).unwrap_err().to_string();
    assert!(said.contains("tangent along this edge"), "{said}");
    let said = bevel(&flat, edge, 1.0, k).unwrap_err().to_string();
    assert!(said.contains("tangent along this edge"), "{said}");
}

/// The ridge 4, 0.25, 9 (a waist between its first and second section),
/// moved down so that its least height above the eaves is `waist`: with
/// the body its ridge edge, the edge as the oracles read it and the
/// station of the waist.
fn waisted(frame: &Frame, k: f64, waist: f64) -> (Body, EdgeId, Roof, f64) {
    let heights = [4.0, 0.25, 9.0];
    let body = ridge(frame, k, &heights, 25.0, false);
    let edge = ridge_edges(&body, frame, k)[0];
    let roof = Roof::of(&body, edge, frame, k);
    let floor = roof.height(roof.lowest(0.0, 50.0 * k)).0 / k;
    let moved: Vec<f64> = heights.iter().map(|h| h - floor + waist).collect();
    let body = ridge(frame, k, &moved, 25.0, false);
    let edge = ridge_edges(&body, frame, k)[0];
    let roof = Roof::of(&body, edge, frame, k);
    let at = roof.lowest(0.0, 50.0 * k);
    let least = roof.height(at).0;
    assert!(
        (least - waist * k).abs() <= 1e-9 * k,
        "the waist is {least} high"
    );
    (body, edge, roof, at)
}

/// A ridge that comes down to the eaves and up again without passing
/// below them: tangent in one place, convex on both sides. Where the
/// section between the tangent planes is no wider than the tolerance, or
/// the faces are tangent by the kernel's measure, the fillet is refused
/// with the place and the reason; with a waist a little higher it is not
/// refused as pinched.
#[test]
fn a_ridge_that_touches_the_eaves_inside_is_refused_where_the_fillet_has_no_width() {
    for k in SCALES {
        for frame in frames(k) {
            let tol = tolerance(k).linear;
            // Tangent: every radius.
            let (body, edge, roof, at) = waisted(&frame, k, 0.0);
            for r in [0.3, 1.0, 3.0] {
                let (place, why, places) = pinched(round(&body, edge, r * k, k), "a waist of 0");
                assert_eq!(why, FilletPinch::Tangent);
                assert_eq!(places, 1);
                // The bottom of a parabola is flat: the place to a
                // hundredth of the loft's unit.
                let off = place.distance(roof.point(at));
                assert!(off <= 1e-2 * k, "the place is {off} off");
            }
            // 1e-5 high: the faces turn by 2e-6 there. A fillet of 0.3 is
            // 6e-7 wide, under the tolerance of 1e-6; one of 3 is 6e-6
            // wide and its faces are not tangent by the kernel's measure
            // (a sine of 1e-6).
            let (body, edge, roof, at) = waisted(&frame, k, 1e-5);
            let wide = |r: f64| 2.0 * r * k * roof.half_tan(at);
            assert!(wide(0.3) < 0.7 * tol && wide(3.0) > 5.0 * tol);
            let (place, why, _) = pinched(round(&body, edge, 0.3 * k, k), "a waist of 1e-5");
            match why {
                FilletPinch::Narrow { width, tolerance } => {
                    assert!((width - wide(0.3)).abs() <= 1e-3 * width, "{width}");
                    assert_eq!(tolerance, tol);
                }
                other => panic!("{other:?}"),
            }
            assert!(place.distance(roof.point(at)) <= 1e-2 * k);
            let said = round(&body, edge, 0.3 * k, k).unwrap_err().to_string();
            assert!(said.contains("no wider there than the tolerance"), "{said}");
            assert!(said.contains("a larger radius is wider there"), "{said}");
            assert!(
                !matches!(
                    round(&body, edge, 3.0 * k, k),
                    Err(KernelError::FilletPinched { .. })
                ),
                "a fillet six tolerances wide at its waist is not pinched"
            );
        }
    }
}

/// A waist that stays clear of tangent is built as it was: the fillet is
/// held against the oracle of the module documentation, its two borders
/// keep to their own sides and stay apart by what the section between the
/// tangent planes says, and the body can be measured.
#[test]
fn a_fillet_that_narrows_inside_its_edge_without_closing_is_built_right() {
    let cases: Vec<(f64, usize, f64, f64)> = vec![
        // (scale, frame, waist, radius)
        (1.0, 0, 1e-3, 0.3),
        (1.0, 0, 1e-3, 1.0),
        (1.0, 0, 1e-3, 3.0),
        (1.0, 0, 1e-2, 0.3),
        (1.0, 0, 1e-2, 3.0),
        (1e-3, 0, 3e-3, 1.0),
        (1.0, 1, 1e-2, 1.0),
        (1.0, 2, 3e-2, 0.3),
        (1e-3, 2, 3e-2, 3.0),
        (1e6, 1, 1e-2, 0.3),
        (1e6, 2, 0.1, 1.0),
    ];
    for (k, frame, waist, r) in cases {
        let frame = frames(k)[frame];
        let tol = tolerance(k);
        let (body, edge, roof, at) = waisted(&frame, k, waist);
        let made = round(&body, edge, r * k, k).unwrap();
        assert_eq!(check(&made.body, &tol), []);
        let removed = mass(&body).volume - mass(&made.body).volume;
        let oracle = fillet_removed(&roof, r * k);
        assert!(
            (removed - oracle).abs() <= within(&roof, 1.5 * r * k),
            "waist {waist}, radius {r}, scale {k}: {removed} against {oracle}"
        );
        let (least, apart) = borders(&made, &roof);
        let wide = 2.0 * r * k * roof.half_tan(at);
        assert!(apart, "the borders cross");
        assert!(
            (least - wide).abs() <= 0.02 * wide,
            "waist {waist}, radius {r}: {least} wide against {wide}"
        );
    }
}

/// A ridge that comes down towards the eaves at its far end and stops
/// short of them by `end`: the fillet narrows towards its end, where the
/// end face cuts it off, and is built as it was, against the oracle. With
/// the end 0.0005 high the faces meet at 0.0057° there and a fillet of 0.3
/// ends 3e-5 wide.
#[test]
fn a_fillet_that_narrows_to_the_end_of_its_edge_is_built_as_before() {
    let mut cases: Vec<(f64, usize, f64, f64)> = Vec::new();
    for end in [0.5, 0.05, 0.005, 0.0005] {
        for r in [0.3, 1.0, 3.0] {
            cases.push((1.0, 0, end, r));
        }
    }
    cases.push((1.0, 1, 0.0005, 0.3));
    cases.push((1.0, 2, 0.05, 3.0));
    cases.push((1e-3, 1, 0.05, 1.0));
    cases.push((1e6, 2, 0.05, 1.0));
    cases.push((1e6, 0, 0.005, 3.0));
    for (k, frame, end, r) in cases {
        let frame = frames(k)[frame];
        let tol = tolerance(k);
        let body = ridge(&frame, k, &[6.0, end], 40.0, true);
        let edge = ridge_edges(&body, &frame, k)[0];
        let roof = Roof::of(&body, edge, &frame, k);
        let made = match round(&body, edge, r * k, k) {
            Ok(made) => made,
            // As before: the largest radius at the flattest end.
            Err(KernelError::Fillet { .. }) if end == 0.0005 && r == 3.0 => continue,
            Err(other) => panic!("end {end}, radius {r}, scale {k}: {other}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let removed = mass(&body).volume - mass(&made.body).volume;
        let oracle = fillet_removed(&roof, r * k);
        assert!(
            (removed - oracle).abs() <= within(&roof, 1.5 * r * k),
            "end {end}, radius {r}, scale {k}: {removed} against {oracle}"
        );
        let (_, apart) = borders(&made, &roof);
        assert!(apart, "the borders cross");
    }
}

/// The ends of an edge are not places inside it. A ridge that reaches the
/// eaves exactly at its end is refused as before, by the construction
/// (a fillet that fades out at such an end is not built yet); one that
/// passes below them a little before its end is convex on one side of
/// that place and concave on the other, and is refused with it.
#[test]
fn the_end_of_an_edge_is_not_a_place_inside_it() {
    for k in SCALES {
        let frame = frames(k)[1];
        let body = ridge(&frame, k, &[6.0, 0.0], 40.0, true);
        let edge = ridge_edges(&body, &frame, k)[0];
        for r in [0.3, 1.0, 3.0] {
            let result = round(&body, edge, r * k, k);
            assert!(
                matches!(result, Err(KernelError::Fillet { .. })),
                "a ridge tangent at its end, radius {r}, scale {k}: {:?}",
                result.map(|_| "built")
            );
        }
        // Below the eaves over the last 0.0033 of 40.
        let body = ridge(&frame, k, &[6.0, -0.0005], 40.0, true);
        let edge = ridge_edges(&body, &frame, k)[0];
        let roof = Roof::of(&body, edge, &frame, k);
        let turns = roof.zero(0.0, 40.0 * k);
        assert!((40.0 * k - turns - 0.0033 * k).abs() <= 1e-4 * k);
        for r in [0.3, 1.0, 3.0] {
            let (at, why, places) = pinched(round(&body, edge, r * k, k), "below at the end");
            assert_eq!(why, FilletPinch::Flips);
            assert_eq!(places, 1);
            assert!(at.distance(roof.point(turns)) <= 1e-5 * k);
        }
    }
}

/// Chamfers of these ridges are built as they were, through the places
/// where the faces are tangent and from ridge into valley: each is held
/// against the section in closed form (the module documentation), which
/// asks nothing of how the chamfer was built. On the ruled roofs the
/// volume itself is in closed form.
#[test]
fn chamfers_of_the_ridges_against_their_sections_in_closed_form() {
    let shapes: Vec<(&str, Vec<f64>, f64, bool)> = vec![
        ("6 to 0, ruled", vec![6.0, 0.0], 40.0, true),
        ("6 to 0.0005, ruled", vec![6.0, 0.0005], 40.0, true),
        ("6 to -1, ruled", vec![6.0, -1.0], 40.0, true),
        ("6 2 0 2", vec![6.0, 2.0, 0.0, 2.0], 15.0, false),
        ("6 3 -3", vec![6.0, 3.0, -3.0], 25.0, false),
        ("6 2.5 0.6 0", vec![6.0, 2.5, 0.6, 0.0], 15.0, false),
    ];
    for (k, frame) in [(1.0, 0), (1.0, 2), (1e-3, 1), (1e6, 2)] {
        let frame = frames(k)[frame];
        let tol = tolerance(k);
        for (name, heights, step, ruled) in &shapes {
            let body = ridge(&frame, k, heights, *step, *ruled);
            let edge = ridge_edges(&body, &frame, k)[0];
            let roof = Roof::of(&body, edge, &frame, k);
            for d in [0.3, 1.0, 2.0] {
                let made = match bevel(&body, edge, d * k, k) {
                    Ok(made) => made,
                    Err(error) => panic!("{name}, distance {d}, scale {k}: {error}"),
                };
                assert_eq!(check(&made, &tol), []);
                let removed = mass(&body).volume - mass(&made).volume;
                let oracle = chamfer_removed(&roof, d * k);
                if *ruled {
                    let closed =
                        chamfer_removed_ruled(heights[0] * k, heights[1] * k, step * k, d * k, k);
                    assert!(
                        (oracle - closed).abs() <= 1e-9 * closed.abs(),
                        "{name}: the section integrated {oracle}, in closed form {closed}"
                    );
                }
                assert!(
                    (removed - oracle).abs() <= within(&roof, 2.0 * d * k),
                    "{name}, distance {d}, scale {k}: {removed} against {oracle}"
                );
            }
        }
    }
}

/// Seeded ridges of two to five sections, ruled and smooth, with heights
/// that pass through zero, touch it, come near it or stay clear of it, at
/// three scales and in three frames. What the kernel answers is held
/// against the ridge edge read by the test (the tangent of half the turn
/// at 4000 stations): an edge that turns the other way at a station
/// inside it, or whose least turn inside is under half the bound, has to
/// be refused as pinched, at a place where the test finds the turn under
/// the bound too; one whose turn stays over twice the bound all the way
/// must not be; and whatever is built of those can be measured and has its
/// borders apart.
#[test]
fn seeded_ridges_are_refused_exactly_where_they_are_pinched() {
    let mut rng = Rng(0x51de_c0de);
    let (mut refused, mut built, mut otherwise) = (0, 0, 0);
    for case in 0..96 {
        let k = SCALES[rng.below(3) as usize];
        let frame = frames(k)[rng.below(3) as usize];
        let tol = tolerance(k).linear;
        let count = 2 + rng.below(4) as usize;
        let ruled = count == 2 || rng.below(3) == 0;
        let step = 40.0 / (count - 1) as f64;
        // Heights between −2 and 6, some of them zero or next to it.
        let heights: Vec<f64> = (0..count)
            .map(|_| match rng.below(6) {
                0 => 0.0,
                1 => [1e-6, 1e-4, 1e-2, -1e-4][rng.below(4) as usize],
                2 => -2.0 * rng.unit(),
                _ => 0.2 + 5.8 * rng.unit(),
            })
            .collect();
        let r = [0.1, 0.3, 1.0, 3.0][rng.below(4) as usize];
        let body = ridge(&frame, k, &heights, step, ruled);
        for edge in ridge_edges(&body, &frame, k) {
            let roof = Roof::of(&body, edge, &frame, k);
            let n = 4000;
            let at = |i: usize| roof.length * i as f64 / n as f64;
            let middle = roof.half_tan(0.5 * roof.length);
            let sign = if middle < 0.0 { -1.0 } else { 1.0 };
            let turn: Vec<f64> = (0..=n).map(|i| sign * roof.half_tan(at(i))).collect();
            // The bound as a tangent of half the turn: the section as wide
            // as the tolerance, or a sine of 1e-6.
            let bound = (0.5 * tol / (r * k)).max(0.5e-6);
            let lowest = (1..n).min_by(|a, b| turn[*a].total_cmp(&turn[*b])).unwrap();
            // The stretch of stations under the bound about the lowest:
            // one that reaches an end of the edge is that end's.
            let mut stretch = (lowest, lowest);
            while stretch.0 > 0 && turn[stretch.0 - 1] <= bound {
                stretch.0 -= 1;
            }
            while stretch.1 < n && turn[stretch.1 + 1] <= bound {
                stretch.1 += 1;
            }
            let at_end = stretch.0 == 0 || stretch.1 == n;
            let opposite = (1..n).any(|i| turn[i] < -bound);
            let result = round(&body, edge, r * k, k);
            let what =
                format!("case {case}: heights {heights:?}, ruled {ruled}, radius {r}, scale {k}");
            if middle.abs() <= 0.5e-6 {
                // Tangent at the middle: refused there, or as tangent all
                // along.
                assert!(result.is_err(), "{what}");
                refused += 1;
            } else if opposite || (!at_end && turn[lowest] <= 0.5 * bound) {
                let (found, why, _) = pinched(result, &what);
                let x = roof.station(found);
                assert!(x > tol && x < roof.length - tol, "{what}: at an end");
                let there = sign * roof.half_tan(x);
                assert!(
                    there <= 1.001 * bound,
                    "{what}: the turn is {there} at the place"
                );
                // Convex on one side of the place and concave on the
                // other, or not.
                let beside = 1e-3 * k;
                let flips = roof.half_tan(x - beside) * roof.half_tan(x + beside) < 0.0;
                assert_eq!(why == FilletPinch::Flips, flips, "{what}: {why:?}");
                refused += 1;
            } else if turn.iter().all(|t| *t >= 2.0 * bound) {
                match result {
                    Ok(made) => {
                        assert!(mass_properties(&made.body).is_ok(), "{what}");
                        let (least, apart) = borders(&made, &roof);
                        assert!(apart && least > tol, "{what}: borders {least} apart");
                        assert_eq!(check(&made.body, &tolerance(k)), [], "{what}");
                        built += 1;
                    }
                    Err(KernelError::FilletPinched { .. }) => {
                        panic!(
                            "{what}: refused as pinched, with a turn of {} at least",
                            turn[lowest]
                        )
                    }
                    Err(_) => otherwise += 1,
                }
            }
        }
    }
    println!("refused as pinched {refused}, built {built}, refused otherwise {otherwise}");
    assert!(
        refused >= 40 && built >= 10,
        "{refused} refused, {built} built"
    );
}
