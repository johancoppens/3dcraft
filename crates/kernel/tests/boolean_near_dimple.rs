//! Two faces that come within twice the tolerance of each other at a point
//! inside both, found wherever that point is (2026-11-25;
//! docs/design/booleans.md › *Near approaches inside an edge or a face* ›
//! *The dimple*).
//!
//! The second review of near approaches found a wrong body the look missed
//! (and `main` built before the look): a plate less a ball of radius 20
//! whose lowest point is 1.5 tolerances over the plate's bottom, off the
//! plate's middle. The look's search between two faces (alternating
//! projections from three seeds) crawls there, by `1 − gap / radius` a
//! step, and stopped 0.22 units short. Now the pairs of points square to
//! both surfaces are taken in closed form where the kinds have them (a
//! plane or a ball against any analytic surface, two cylinders), and the
//! other kinds (a spline face among them) are searched on by a descent
//! where the projections have not settled (`near.rs` › `least_between`).
//!
//! - The dimple at several places and radii (6, 20 and 100), its web 1.5
//!   tolerances thick refused, 3 tolerances one shell, checked.
//! - The same dimple under a plate whose bottom is a flat spline face: a
//!   spline against a ball, searched, not in closed form.
//! - The other pairs in closed form, as two parts whose nearest points lie
//!   inside a face of each, away from every edge: a ring tilted over a
//!   plate, a ball beside a rod's round side, beside a cone's side and
//!   beside a ring, and a ball in a spherical cavity a little larger than
//!   itself (two spheres nearly tangent, where the projections crawl as at
//!   the dimple). In the band the union is refused and the interference
//!   is not clear; at 3 tolerances the union is built, checked, and the
//!   interference clear.
//!
//! All at 1e-3, 1 and 1e3, as found and turned.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, Interference, boolean, interference};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, check};

/// The scales the cases are built at.
const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

/// The tolerance at scale `k`: 0.01 at 1, against bodies of 2 to 30 units
/// (the near-approach tests').
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(0.01 * k, 1e-9).unwrap()
}

/// The frames the bodies are built in, at scale `k`: the world's, and
/// turned (the near-approach tests' frame).
fn frames(k: f64) -> Vec<(&'static str, Frame)> {
    vec![
        ("as found", Frame::WORLD),
        (
            "turned",
            Frame::new(
                Vec3::new(13.0 * k, -7.0 * k, 3.0 * k),
                Vec3::new(0.3, -0.5, 0.81),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap(),
        ),
    ]
}

/// What a boolean said, in a word.
fn said(r: &Result<BooleanResult, KernelError>) -> String {
    match r {
        Ok(r) => format!("built ({} shells)", r.body.shells.len()),
        Err(KernelError::ZeroThickness { .. }) => "zero thickness".to_string(),
        Err(KernelError::Unsupported { reason, .. }) => format!("refused: {reason}"),
        Err(e) => format!("error: {e}"),
    }
}

/// A refusal of the band: too near to tell apart, or zero thickness.
fn refused(r: &Result<BooleanResult, KernelError>) -> bool {
    matches!(
        r,
        Err(KernelError::Unsupported { .. } | KernelError::ZeroThickness { .. })
    )
}

/// Built as `shells` shells and checked.
fn built(r: &Result<BooleanResult, KernelError>, shells: usize, tol: &Tolerance) -> bool {
    r.as_ref()
        .is_ok_and(|r| r.body.shells.len() == shells && check(&r.body, tol).is_empty())
}

/// The interference in a word.
fn contact(found: &Result<Interference, KernelError>) -> &'static str {
    match found {
        Ok(Interference::Clear) => "clear",
        Ok(Interference::Touch { .. }) => "touch",
        Ok(Interference::Overlap(_)) => "overlap",
        Err(_) => "unknown",
    }
}

/// A plate 10 by 10 by 10 at scale `k` in `frame` less a ball of radius
/// `radius` whose lowest point lies `gap` tolerances over the plate's
/// bottom (`y = 0`) at `(x, z)`: a dish whose bottom is a web that thin
/// there. The ball meets the plate's sides at least 0.08 over the bottom
/// (eight tolerances); it has no edge and no vertex.
fn dish(k: f64, frame: &Frame, tol: &Tolerance, radius: f64, gap: f64, place: (f64, f64)) -> Body {
    let centre = frame.point(place.0 * k, radius * k + gap * tol.linear, place.1 * k);
    primitives::sphere(&frame.at(centre).unwrap(), radius * k).unwrap()
}

/// The dimples: a radius and the place `(x, z)` of the lowest point, each
/// side the ball meets at least 0.08 over the bottom (`r − √(r² − d²)`, `d`
/// the place's distance from the side).
const DIMPLES: [(f64, (f64, f64)); 8] = [
    (6.0, (2.0, 5.0)),
    (6.0, (5.0, 5.0)),
    (6.0, (7.5, 3.0)),
    (20.0, (2.0, 5.0)),
    (20.0, (5.0, 5.0)),
    (20.0, (7.5, 3.0)),
    (100.0, (5.0, 5.0)),
    (100.0, (6.0, 4.0)),
];

/// The second review's web under a dimple, at more places and radii: the
/// lowest point of a ball of radius 6, 20 or 100 1.5 tolerances over the
/// plate's bottom, under the middle, off it, and towards a corner. The
/// nearest points of the bottom's plane and the ball are the foot of the
/// centre and the point of the ball over it, in closed form, wherever the
/// seeds of the search lie: refused as too near to tell apart. At 3
/// tolerances one shell, checked. At 1e-3, 1 and 1e3, as found and
/// turned.
#[test]
fn a_web_in_the_band_under_a_dimple_is_refused_wherever_it_is() {
    let mut faults = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (name, frame) in frames(k) {
            let plate = primitives::block(&frame, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
            for (radius, place) in DIMPLES {
                for gap in [1.5, 3.0] {
                    let ball = dish(k, &frame, &tol, radius, gap, place);
                    let r = boolean(&plate, &ball, BooleanOp::Subtract, &tol);
                    let what = format!(
                        "a web {gap} tolerances thick under a dimple of radius {radius} at {place:?}, at {k}, {name}"
                    );
                    println!("near_dimple: {what}: the subtraction {}", said(&r));
                    let right = if gap < 2.0 {
                        refused(&r)
                    } else {
                        built(&r, 1, &tol)
                    };
                    if !right {
                        faults.push(format!("{what}: the subtraction {}", said(&r)));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// `body` with the face that lies in the plane through `at` square to
/// `normal` given a spline surface on that plane: a bicubic net over the
/// face's extent, so the face, its edges and its vertices stay where they
/// were and the surface is freeform (`boolean_corner_beside.rs`'s).
fn spline_on(mut body: Body, at: Vec3, normal: Vec3, tol: &Tolerance) -> Body {
    let index = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                let f = *plane.frame();
                f.z().cross(normal).length() <= 1e-9
                    && (at - f.origin()).dot(f.z()).abs() <= 1e-3 * tol.linear
            }
            _ => false,
        })
        .expect("the face on the plane");
    let Surface::Plane(plane) = &body.faces[index].surface else {
        panic!("a plane");
    };
    let f = *plane.frame();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for vertex in &body.vertices {
        let (x, y, z) = f.local(vertex.point);
        if z.abs() <= tol.linear {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    let mut ctrl = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            ctrl.push(f.point(
                lo[0] + (hi[0] - lo[0]) * f64::from(i) / 3.0,
                lo[1] + (hi[1] - lo[1]) * f64::from(j) / 3.0,
                0.0,
            ));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    body.faces[index].surface = Surface::Nurbs(
        NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, vec![1.0; 16]).unwrap(),
    );
    assert_eq!(check(&body, tol), [], "the body with a spline face");
    body
}

/// The dimple of radius 20 under the plate's middle and off it, the
/// plate's bottom a flat spline face: a spline against a ball, which has
/// no closed form; the projections crawl as against the plane, and the
/// descent from where they stopped finds the lowest point. Refused in the
/// band; one shell at 3 tolerances, checked. At 1e-3, 1 and 1e3, as found
/// and turned.
#[test]
fn a_web_in_the_band_over_a_spline_bottom_is_refused() {
    let mut faults = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (name, frame) in frames(k) {
            let plate = primitives::block(&frame, 10.0 * k, 10.0 * k, 10.0 * k).unwrap();
            let plate = spline_on(plate, frame.origin(), frame.y(), &tol);
            for place in [(2.0, 5.0), (5.0, 5.0), (7.5, 3.0)] {
                for gap in [1.5, 3.0] {
                    let ball = dish(k, &frame, &tol, 20.0, gap, place);
                    let r = boolean(&plate, &ball, BooleanOp::Subtract, &tol);
                    let what = format!(
                        "a web {gap} tolerances thick over a spline bottom under a dimple of radius 20 at {place:?}, at {k}, {name}"
                    );
                    println!("near_dimple: {what}: the subtraction {}", said(&r));
                    let right = if gap < 2.0 {
                        refused(&r)
                    } else {
                        built(&r, 1, &tol)
                    };
                    if !right {
                        faults.push(format!("{what}: the subtraction {}", said(&r)));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}

/// The kinds of two parts whose nearest points lie inside a face of each.
#[derive(Clone, Copy, Debug)]
enum Pair {
    /// A ring (major radius 3, minor 1) tilted 30 degrees over a plate.
    RingOverPlate,
    /// A ball of radius 1.5 over the middle of a rod of radius 1.
    BallBesideRod,
    /// A ball of radius 1 beside a cone's side, half way up.
    BallBesideCone,
    /// A ball of radius 1.2 beside a ring's tube, 40 degrees up it.
    BallBesideRing,
    /// A ball of radius 9 in a cavity of radius 10, off its middle.
    BallInCavity,
}

/// The two bodies of `kind`, `gap` tolerances apart at their nearest
/// points, at scale `k` in `frame`, and the shells their union has when
/// they are apart.
fn pair(kind: Pair, k: f64, frame: &Frame, tol: &Tolerance, gap: f64) -> (Body, Body, usize) {
    let at = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
    let dir = |x: f64, y: f64, z: f64| frame.vector(x, y, z).normalized().unwrap();
    let g = gap * tol.linear / k;
    let ball =
        |centre: Vec3, r: f64| primitives::sphere(&frame.at(centre).unwrap(), r * k).unwrap();
    match kind {
        Pair::RingOverPlate => {
            let plate = primitives::block(
                &frame.at(at(-8.0, -8.0, -2.0)).unwrap(),
                16.0 * k,
                16.0 * k,
                2.0 * k,
            )
            .unwrap();
            let (s, c) = 30.0_f64.to_radians().psin_cos();
            // The lowest point `R sin 30 + r` under the centre.
            let centre = at(1.0, -2.0, 3.0 * s + 1.0 + g);
            let ring = primitives::torus(
                &Frame::new(centre, dir(0.0, -s, c), frame.x()).unwrap(),
                3.0 * k,
                k,
            )
            .unwrap();
            (plate, ring, 2)
        }
        Pair::BallBesideRod => {
            let rod = primitives::cylinder(
                &Frame::new(at(-5.0, 0.0, 0.0), frame.x(), frame.y()).unwrap(),
                k,
                10.0 * k,
            )
            .unwrap();
            (rod, ball(at(0.7, 0.0, 2.5 + g), 1.5), 2)
        }
        Pair::BallBesideCone => {
            // A cone of radius 3 and height 6: half way up, its side at
            // (1.5, 0, 3), its outward normal (2, 0, 1) / √5 there.
            let cone =
                primitives::cone(&frame.at(at(0.0, 0.0, 0.0)).unwrap(), 3.0 * k, 6.0 * k).unwrap();
            let out = dir(2.0, 0.0, 1.0);
            (
                cone,
                ball(at(1.5, 0.0, 3.0) + out * ((1.0 + g) * k), 1.0),
                2,
            )
        }
        Pair::BallBesideRing => {
            let ring =
                primitives::torus(&frame.at(at(0.0, 0.0, 0.0)).unwrap(), 4.0 * k, k).unwrap();
            let (s25, c25) = 25.0_f64.to_radians().psin_cos();
            let (s40, c40) = 40.0_f64.to_radians().psin_cos();
            let middle = at(4.0 * c25, 4.0 * s25, 0.0);
            let out = dir(c40 * c25, c40 * s25, s40);
            (ring, ball(middle + out * ((1.0 + 1.2 + g) * k), 1.2), 2)
        }
        Pair::BallInCavity => {
            let block = primitives::block(
                &frame.at(at(-15.0, -15.0, -15.0)).unwrap(),
                30.0 * k,
                30.0 * k,
                30.0 * k,
            )
            .unwrap();
            let hollow = boolean(
                &block,
                &ball(at(0.0, 0.0, 0.0), 10.0),
                BooleanOp::Subtract,
                tol,
            )
            .unwrap()
            .body;
            let off = dir(1.0, 2.0, 2.0) * ((1.0 - g) * k);
            (hollow, ball(at(0.0, 0.0, 0.0) + off, 9.0), 3)
        }
    }
}

/// Two parts whose nearest points lie inside a face of each, of the kinds
/// whose pairs square to both surfaces are in closed form (a plane and a
/// ring, a ball and a rod, a cone, a ring, a ball): 1.5 tolerances apart
/// the union is refused (zero thickness) and the interference is a touch,
/// or unknown; 3 tolerances apart the union is the two parts, checked, and
/// the interference clear. At 1e-3, 1 and 1e3, as found and turned.
#[test]
fn two_parts_in_the_band_at_a_point_inside_their_faces_are_not_united() {
    let mut faults = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (name, frame) in frames(k) {
            for kind in [
                Pair::RingOverPlate,
                Pair::BallBesideRod,
                Pair::BallBesideCone,
                Pair::BallBesideRing,
                Pair::BallInCavity,
            ] {
                for gap in [1.5, 3.0] {
                    let (a, b, shells) = pair(kind, k, &frame, &tol, gap);
                    let r = boolean(&a, &b, BooleanOp::Union, &tol);
                    let found = interference(&a, &b, &tol);
                    let what = format!("{kind:?} {gap} tolerances apart, at {k}, {name}");
                    println!(
                        "near_dimple: {what}: the union {}, the interference {}",
                        said(&r),
                        contact(&found)
                    );
                    let right = if gap < 2.0 {
                        refused(&r) && matches!(contact(&found), "touch" | "unknown")
                    } else {
                        built(&r, shells, &tol) && contact(&found) == "clear"
                    };
                    if !right {
                        faults.push(format!(
                            "{what}: the union {}, the interference {}",
                            said(&r),
                            contact(&found)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} wrong answers:\n  {}",
        faults.len(),
        faults.join("\n  ")
    );
}
