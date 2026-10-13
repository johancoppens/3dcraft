//! How long `mass_properties` takes on freeform faces whose boundary holds
//! spline edges: the domed block of the test kit on its own and after a
//! union with a post, whose roof is then bounded by the curves in which
//! the post meets it (cubic Hermite splines with hundreds of knot spans).
//! Ignored by default: it prints timings (the least of [`REPEATS`] runs,
//! the machine being shared), it asserts nothing but that the numbers
//! come out. It lives here because `topo` cannot see the kernel.
//!
//! `cargo test -p craft3d-testkit --release --test mass_timing -- --ignored --nocapture`

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::Curve3;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::{Body, BodyKind, mass_properties};
use std::time::Instant;

/// Every body of the harness, named.
fn bodies() -> Vec<(String, Body)> {
    let tol = Tolerance::default();
    let post_frame = Frame::new(Vec3::new(2.0, 1.5, -1.0), Vec3::Z, Vec3::X).unwrap();
    let post = primitives::cylinder(&post_frame, 1.2, 6.0).unwrap();
    let mut bodies = Vec::new();
    for (name, nurbs) in [("bezier", false), ("nurbs", true)] {
        let dome = domed_block(&Frame::WORLD, 4.0, 3.0, 2.0, nurbs)
            .unwrap()
            .body;
        let union = boolean(&dome, &post, BooleanOp::Union, &tol).unwrap().body;
        bodies.push((format!("dome ({name} roof)"), dome));
        bodies.push((format!("dome ({name} roof) + post"), union));
    }
    bodies
}

/// `f()` and the least it took over [`REPEATS`] runs: the machine is
/// shared, and the least is the measurement.
fn timed<T>(mut f: impl FnMut() -> T) -> (T, std::time::Duration) {
    let mut least = std::time::Duration::MAX;
    let mut result = None;
    for _ in 0..REPEATS {
        let started = Instant::now();
        let value = f();
        least = least.min(started.elapsed());
        result = Some(value);
    }
    (result.unwrap(), least)
}

const REPEATS: usize = 7;

/// The body with one face only, as a sheet: what one face costs.
fn one_face(body: &Body, index: usize) -> Body {
    let mut single = body.clone();
    single.kind = BodyKind::Sheet;
    single.faces = vec![body.faces[index].clone()];
    single.shells.clear();
    single
}

#[test]
#[ignore = "timing harness: run with --ignored --nocapture"]
fn mass_properties_of_freeform_faces() {
    for (name, body) in bodies() {
        let (mass, whole) = timed(|| mass_properties(&body).unwrap());
        println!(
            "{name}: {} faces, {:.1} ms: volume {:.17e}, area {:.17e}, centroid {:?}",
            body.faces.len(),
            whole.as_secs_f64() * 1e3,
            mass.volume,
            mass.area,
            mass.centroid.unwrap()
        );
        for index in 0..body.faces.len() {
            let single = one_face(&body, index);
            let (face, took) = timed(|| mass_properties(&single).unwrap());
            let mut edges = 0;
            let mut spans = 0;
            for l in &body.faces[index].loops {
                for coedge in &body.loop_(*l).unwrap().coedges {
                    edges += 1;
                    if let Curve3::Nurbs(c) = &body.edge(coedge.edge).unwrap().curve {
                        let mut knots = c.knots().to_vec();
                        knots.dedup();
                        spans += knots.len() - 1;
                    }
                }
            }
            println!(
                "  face {index:2} {:14} {edges:3} edges, {spans:4} spline spans {:8.1} ms  area {:.17e}",
                body.faces[index].surface.kind(),
                took.as_secs_f64() * 1e3,
                face.area
            );
        }
    }
}
