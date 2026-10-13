//! Boolean results through the tessellator (docs/testing.md, oracle 4): an
//! independent check that the bodies are what their mass properties say.
//! The mesh of every result is closed and its volume converges to the
//! closed form.
//!
//! This lives in `testkit` because `kernel` and `tess` share a layer and
//! may not depend on each other, not even for tests.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::PI;

fn boxed(lo: [f64; 3], hi: [f64; 3]) -> Body {
    let frame = Frame::WORLD.at(Vec3::new(lo[0], lo[1], lo[2])).unwrap();
    primitives::block(&frame, hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]).unwrap()
}

fn rod(base: [f64; 3], radius: f64, height: f64) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(base[0], base[1], base[2]))
        .unwrap();
    primitives::cylinder(&frame, radius, height).unwrap()
}

/// Booleans with their volume in closed form.
fn cases() -> Vec<(&'static str, Body, f64)> {
    let tol = Tolerance::default();
    let run = |a: &Body, b: &Body, op| boolean(a, b, op, &tol).unwrap().body;
    let plate = boxed([0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
    let post = rod([0.0, 0.0, 0.0], 1.0, 4.0);
    let half = (0.5_f64).acos();
    let lens = 2.0 * (half - half.sin() * half.cos());
    let tilt = 0.6_f64;
    let slanted = Frame::new(
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::new(tilt.sin(), 0.0, tilt.cos()),
        Vec3::Y,
    )
    .unwrap();
    let above = primitives::block(
        &slanted.at(slanted.point(-10.0, -10.0, 0.0)).unwrap(),
        20.0,
        20.0,
        20.0,
    )
    .unwrap();
    let mut drilled = plate.clone();
    for (x, y) in [(1.0, 1.0), (3.0, 1.0), (2.0, 2.25)] {
        drilled = run(&drilled, &rod([x, y, 0.0], 0.4, 1.0), BooleanOp::Subtract);
    }
    vec![
        (
            "a plate with a hole through it",
            run(
                &plate,
                &rod([1.5, 1.25, -1.0], 0.75, 3.0),
                BooleanOp::Subtract,
            ),
            12.0 - PI * 0.75 * 0.75,
        ),
        (
            "a plate with a blind hole",
            run(
                &plate,
                &rod([2.0, 1.5, 0.25], 0.5, 3.0),
                BooleanOp::Subtract,
            ),
            12.0 - PI * 0.25 * 0.75,
        ),
        (
            "a post on a plate",
            run(&plate, &rod([2.0, 1.5, 1.0], 0.5, 3.0), BooleanOp::Union),
            12.0 + PI * 0.25 * 3.0,
        ),
        (
            "a plate with three holes, cutters flush",
            drilled,
            12.0 - 3.0 * PI * 0.16,
        ),
        (
            "two boxes",
            run(
                &boxed([0.0, 0.0, 0.0], [4.0, 3.0, 2.0]),
                &boxed([1.0, 1.0, 1.0], [6.0, 5.0, 4.0]),
                BooleanOp::Union,
            ),
            24.0 + 60.0 - 6.0,
        ),
        (
            "a beam in two parts",
            run(
                &boxed([-0.5, -4.0, -0.5], [0.5, 4.0, 0.5]),
                &boxed([-3.0, -1.0, -1.0], [3.0, 1.0, 1.0]),
                BooleanOp::Subtract,
            ),
            6.0,
        ),
        (
            "a box with a cavity",
            run(
                &boxed([0.0, 0.0, 0.0], [4.0, 4.0, 4.0]),
                &rod([2.0, 2.0, 1.0], 1.0, 2.0),
                BooleanOp::Subtract,
            ),
            64.0 - 2.0 * PI,
        ),
        (
            "a quarter of a cylinder",
            run(
                &post,
                &boxed([0.0, 0.0, -1.0], [3.0, 3.0, 5.0]),
                BooleanOp::Intersect,
            ),
            PI,
        ),
        (
            "the lens of two cylinders",
            run(
                &rod([0.0, 0.0, 0.0], 1.0, 2.0),
                &rod([1.0, 0.0, 0.0], 1.0, 2.0),
                BooleanOp::Intersect,
            ),
            lens * 2.0,
        ),
        (
            "a cylinder cut at a slant",
            run(&post, &above, BooleanOp::Subtract),
            2.0 * PI,
        ),
        (
            "a cylinder with a slot through its wall",
            run(
                &post,
                &boxed([-3.0, -0.25, 1.0], [3.0, 0.25, 2.0]),
                BooleanOp::Subtract,
            ),
            {
                // The slot's section: the strip |y| <= 1/4 of the unit disc.
                let y: f64 = 0.25;
                4.0 * PI - 2.0 * (y * (1.0 - y * y).sqrt() + y.asin())
            },
        ),
    ]
}

/// Booleans with cones, spheres and tori, and with edges that are splines.
fn curved_cases() -> Vec<(&'static str, Body, f64)> {
    let tol = Tolerance::default();
    let run = |a: &Body, b: &Body, op| boolean(a, b, op, &tol).unwrap().body;
    let at = |x: f64, y: f64, z: f64| Frame::WORLD.at(Vec3::new(x, y, z)).unwrap();
    let ball = primitives::sphere(&at(1.0, -0.5, 3.0), 2.0).unwrap();
    let cap = |r: f64, h: f64| PI * h * h * (3.0 * r - h) / 3.0;
    let sphere = |r: f64| 4.0 / 3.0 * PI * r * r * r;
    let above = boxed([-4.0, -4.0, 3.75], [6.0, 5.0, 9.0]);
    let cone = primitives::cone(&at(1.0, 2.0, 0.0), 1.5, 3.0).unwrap();
    let ring = primitives::torus(&at(0.0, 0.0, 0.0), 3.0, 1.0).unwrap();
    let plate = boxed([0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
    let sink = primitives::cone(
        &Frame::new(Vec3::new(2.0, 1.5, 1.0), -Vec3::Z, Vec3::X).unwrap(),
        1.0,
        0.75,
    )
    .unwrap();
    let dimpled = run(&plate, &sink, BooleanOp::Subtract);
    // Curves without a closed form: splines.
    let upright = rod([0.0, 0.0, -2.0], 1.0, 4.0);
    let lying = primitives::cylinder(
        &Frame::new(Vec3::new(-3.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
        1.0,
        6.0,
    )
    .unwrap();
    let steinmetz = run(&upright, &lying, BooleanOp::Intersect);
    vec![
        (
            "the cap of a ball",
            run(&ball, &above, BooleanOp::Intersect),
            cap(2.0, 1.25),
        ),
        (
            "a ball without its cap",
            run(&ball, &above, BooleanOp::Subtract),
            sphere(2.0) - cap(2.0, 1.25),
        ),
        (
            "an eighth of a ball",
            run(
                &ball,
                &boxed([1.0, -0.5, 3.0], [5.0, 4.0, 8.0]),
                BooleanOp::Intersect,
            ),
            sphere(2.0) / 8.0,
        ),
        (
            "the lens of two balls",
            run(
                &primitives::sphere(&Frame::WORLD, 2.0).unwrap(),
                &primitives::sphere(&at(1.5, 2.0, 0.0), 1.5).unwrap(),
                BooleanOp::Intersect,
            ),
            cap(2.0, 0.4) + cap(1.5, 0.6),
        ),
        (
            "a ball with a hole through it",
            run(
                &primitives::sphere(&at(0.0, 0.0, 3.0), 2.0).unwrap(),
                &rod([0.0, 0.0, 0.0], 1.0, 6.0),
                BooleanOp::Subtract,
            ),
            PI * (2.0 * 3.0_f64.sqrt()).powi(3) / 6.0,
        ),
        (
            "a frustum",
            run(
                &cone,
                &boxed([-4.0, -4.0, 1.0], [6.0, 8.0, 9.0]),
                BooleanOp::Subtract,
            ),
            PI * (2.25 + 1.5 + 1.0) / 3.0,
        ),
        (
            "the tip of a cone",
            run(
                &cone,
                &boxed([-4.0, -4.0, 1.0], [6.0, 8.0, 9.0]),
                BooleanOp::Intersect,
            ),
            PI * 2.0 / 3.0,
        ),
        (
            "half a cone, cut through its apex",
            run(
                &cone,
                &boxed([1.0, -4.0, -1.0], [6.0, 8.0, 9.0]),
                BooleanOp::Intersect,
            ),
            PI * 2.25 * 3.0 / 6.0,
        ),
        (
            "a plate with a conical dimple",
            dimpled.clone(),
            12.0 - PI * 0.75 / 3.0,
        ),
        (
            "a countersunk hole",
            run(
                &dimpled,
                &rod([2.0, 1.5, -1.0], 0.25, 3.0),
                BooleanOp::Subtract,
            ),
            12.0 - PI * 0.5625 * (1.0 + 0.25 + 0.0625) / 3.0 - PI * 0.0625 * 0.4375,
        ),
        (
            "half a ring, cut through its axis",
            run(
                &ring,
                &boxed([0.0, -6.0, -3.0], [6.0, 6.0, 3.0]),
                BooleanOp::Intersect,
            ),
            PI * PI * 3.0,
        ),
        (
            "the upper half of a ring",
            run(
                &ring,
                &boxed([-6.0, -6.0, 0.0], [6.0, 6.0, 3.0]),
                BooleanOp::Intersect,
            ),
            PI * PI * 3.0,
        ),
        (
            "a dome on a plate",
            run(
                &plate,
                &primitives::sphere(&at(2.0, 1.5, 1.0), 1.0).unwrap(),
                BooleanOp::Union,
            ),
            12.0 + sphere(1.0) / 2.0,
        ),
        (
            "a pin with a round end",
            run(
                &rod([2.0, 1.5, 1.0], 0.5, 2.0),
                &primitives::sphere(&at(2.0, 1.5, 1.0), 0.5).unwrap(),
                BooleanOp::Union,
            ),
            PI * 0.5 + sphere(0.5) / 2.0,
        ),
        (
            "what two equal cylinders at right angles share",
            steinmetz,
            16.0 / 3.0,
        ),
    ]
}

#[test]
fn meshes_of_curved_boolean_results_are_closed_and_converge() {
    let tol = Tolerance::default();
    let mut refused = Vec::new();
    let mut meshed = 0;
    for (name, body, volume) in curved_cases() {
        assert_eq!(check(&body, &tol), [], "{name}");
        let exact = mass_properties(&body).unwrap();
        // An edge that is a spline is off by up to the tolerance.
        assert!(
            (exact.volume - volume).abs() < 1e-5 * volume,
            "{name}: {} is not {volume}",
            exact.volume
        );
        let mut previous = f64::INFINITY;
        for chord in [1e-1, 1e-2, 1e-3] {
            let params = TessParams {
                chordal_tolerance: chord,
                ..TessParams::default()
            };
            let mesh = match tessellate_body(&body, &params) {
                Ok(mesh) => mesh,
                Err(e) => {
                    // The tessellator says what it cannot mesh; that is
                    // its limit, recorded below, not a wrong body.
                    refused.push(format!("{name}: {e:?}"));
                    break;
                }
            };
            let welded = mesh.weld().unwrap();
            assert!(welded.is_closed(), "{name}: the mesh is not closed");
            let error = (welded.volume() - volume).abs();
            assert!(
                error <= exact.area * chord,
                "{name} at {chord}: volume {} is not {volume}",
                welded.volume()
            );
            assert!(
                error <= previous + 1e-9 * volume,
                "{name} at {chord}: not converging"
            );
            previous = error;
            if chord == 1e-3 {
                meshed += 1;
            }
        }
    }
    for line in &refused {
        println!("not meshed: {line}");
    }
    println!(
        "{meshed} meshed, {} refused by the tessellator",
        refused.len()
    );
    // What the tessellator refuses today, by name: a change either way
    // shows up here.
    let names: Vec<&str> = refused
        .iter()
        .filter_map(|line| line.split(':').next())
        .collect();
    assert_eq!(names, EXPECTED_REFUSALS, "{refused:#?}");
}

/// Boolean results the tessellator does not mesh: none. (The zone of a
/// sphere between two circles, a ball with a hole through it, was one
/// until the tessellator learnt a sphere face that is everything outside
/// its loops.)
const EXPECTED_REFUSALS: [&str; 0] = [];

#[test]
fn meshes_of_boolean_results_are_closed_and_converge() {
    let tol = Tolerance::default();
    for (name, body, volume) in cases() {
        assert_eq!(check(&body, &tol), [], "{name}");
        let exact = mass_properties(&body).unwrap();
        assert!(
            (exact.volume - volume).abs() < 1e-9 * volume,
            "{name}: {} is not {volume}",
            exact.volume
        );
        let mut previous = f64::INFINITY;
        let mut area_error = f64::INFINITY;
        for chord in [1e-1, 1e-2, 1e-3] {
            let params = TessParams {
                chordal_tolerance: chord,
                ..TessParams::default()
            };
            let mesh = match tessellate_body(&body, &params) {
                Ok(mesh) => mesh,
                Err(e) => panic!("{name}: {e:?}"),
            };
            let welded = mesh.weld().unwrap();
            assert!(welded.is_closed(), "{name}: the mesh is not closed");
            let error = (welded.volume() - volume).abs();
            // A chord error of c over the whole surface, at most.
            assert!(
                error <= exact.area * chord,
                "{name} at {chord}: volume {} is not {volume}",
                welded.volume()
            );
            assert!(
                error <= previous + 1e-12 * volume,
                "{name} at {chord}: not converging"
            );
            // Not "at most the exact area": the flat face around a hole
            // gains what the polygon in place of the circle leaves.
            let error_now = (mesh.area() - exact.area).abs();
            assert!(
                error_now <= area_error + 1e-12 * exact.area,
                "{name} at {chord}: the area is not converging"
            );
            area_error = error_now;
            previous = error;
        }
        // At a chord of 1e-3 on sizes of a few units.
        assert!(area_error <= 1e-2 * exact.area, "{name}: {area_error}");
    }
}
