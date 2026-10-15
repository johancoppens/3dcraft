//! Intersections inside a box: what the box cuts off must not spoil what
//! it keeps (2026-10-16, the lamp's finding B1).
//!
//! Two spline roofs that cross (a domed block of the test kit and one
//! hung upside down over it, tilted) are intersected inside seeded boxes
//! that cut their curve at random places, at three scales. Every curve
//! returned has to be within the tolerance of both surfaces along its
//! whole length, the ends the box made included. Before the pieces a box
//! cuts were refined, the span to such an end was a new cubic held to
//! nothing, and on the lamp's shell and cell the fit was refused ("an
//! intersection curve could not be fitted within the tolerance"; that
//! case, which these roofs do not reproduce, is
//! `crates/engine/tests/boolean_findings.rs`).
//!

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_math::{Bbox3, Frame, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::Body;

/// The domed block upright at the origin and the one hung over it,
/// turned about a tilted axis, at scale `k`.
fn roofs(k: f64) -> (Body, Body) {
    let tilt = |v: Vec3| {
        // The same small turn for both, so nothing is axis-aligned.
        let axis = Vec3::new(0.3, -0.4, 0.86).normalized().unwrap();
        let angle: f64 = 0.37;
        let (s, c) = angle.sin_cos();
        v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
    };
    let upright = Frame::new(
        tilt(Vec3::new(0.0, 0.0, 0.0)) * k,
        tilt(Vec3::Z),
        tilt(Vec3::X),
    )
    .unwrap();
    let hung = Frame::new(
        tilt(Vec3::new(4.6, 0.4, 4.3)) * k,
        tilt(Vec3::new(0.08, -0.05, -1.0)),
        tilt(Vec3::new(-1.0, 0.1, 0.0)),
    )
    .unwrap();
    (
        domed_block(&upright, 4.0 * k, 3.0 * k, 2.0 * k, true)
            .unwrap()
            .body,
        domed_block(&hung, 4.2 * k, 3.4 * k, 2.1 * k, true)
            .unwrap()
            .body,
    )
}

fn roof(body: &Body) -> &Surface {
    &body
        .faces
        .iter()
        .find(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap()
        .surface
}

/// Boxes per scale.
const BOXES: usize = 60;

#[test]
fn boxes_that_cut_the_curve_keep_it_within_the_tolerance() {
    for k in SCALES {
        let tol = tolerance(k);
        let (a, b) = roofs(k);
        let (sa, sb) = (roof(&a), roof(&b));
        let whole = a.bbox().unwrap().corners().unwrap();
        let mut rng = Rng(0x5eed_b0c5 ^ k.to_bits());
        let mut cut = 0;
        for _ in 0..BOXES {
            let corner = |rng: &mut Rng| {
                Vec3::new(
                    rng.between(whole.0.x, whole.1.x),
                    rng.between(whole.0.y, whole.1.y),
                    rng.between(whole.0.z, whole.1.z),
                )
            };
            // A box about a random point, up to a third of the body across.
            let middle = corner(&mut rng);
            let reach = (whole.1 - whole.0) * rng.between(0.02, 0.33);
            let within = Bbox3::from_points([middle - reach, middle + reach]);
            let met = meet_surfaces(sa, sb, &within, &tol)
                .unwrap_or_else(|e| panic!("at {k} in {within:?}: {e}"));
            let SurfaceMeeting::Meets(meeting) = met else {
                continue;
            };
            for curve in &meeting.curves {
                cut += usize::from(!curve.closed);
                assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
                let (t0, t1) = curve.range;
                for i in 0..=200 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 200.0);
                    for s in [sa, sb] {
                        let (u, v) = s.project(p).unwrap();
                        let off = s.eval(u, v).distance(p);
                        assert!(off <= tol.linear, "at {k}: {off} off a roof at {p:?}");
                    }
                }
            }
        }
        assert!(cut > 10, "at {k} the boxes cut the curve {cut} times");
    }
}
