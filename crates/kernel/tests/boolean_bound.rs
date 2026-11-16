//! The bound on the work of an operation that holds trees (a boolean,
//! interference, a classification of points), on the ledger
//! (`craft3d_geom::allowance::asked`): an operation that would need more
//! ends with a refusal that says so and names the faces, never runs on
//! (docs/design/booleans.md › *A boolean that runs for most of an hour*).
//! The bound is set low here (`set_operation_work`), so that a boolean
//! which runs for minutes at the real bound ends in seconds with the
//! refusal it would give there; the bound is process-wide, so this file
//! holds one test, which goes through its cases one after the other.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_geom::{NurbsSurface, Surface, allowance};
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points, set_operation_work};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;
use std::time::Instant;

/// As `boolean_weighted_chart.rs`.
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
                    lo[0] + (hi[0] - lo[0]) * f64::from(i) / 5.0,
                    lo[1] + (hi[1] - lo[1]) * f64::from(j) / 5.0,
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

const TOO_MUCH: &str = "it needs more work than one operation is allowed";

#[test]
fn an_operation_past_its_bound_is_refused_saying_so_and_naming_its_faces() {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    // The long block's seed 4: counted work that runs for minutes; held
    // to a bound of 200 million it ends in seconds, refused.
    let long = block_with_weighted_sides(Vec3::new(30.0, 900.0, 20.0), 4);
    let tool = primitives::block(
        &Frame::WORLD.at(Vec3::new(12.0, 315.0, 9.0)).unwrap(),
        27.0,
        765.0,
        16.0,
    )
    .unwrap();
    set_operation_work(Some(200_000_000));
    let started = Instant::now();
    let (made, asked) =
        allowance::measured_asked(|| boolean(&long, &tool, BooleanOp::Intersect, &tol));
    let took = started.elapsed().as_secs_f64();
    let text = made
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    println!("the long block, seed 4: {text} after {asked} on the ledger, {took:.1} s");
    assert!(text.contains(TOO_MUCH), "{text}");
    assert!(text.contains("face "), "no face named: {text}");
    // Past the bound by at most what one step asks.
    assert!((200_000_000..260_000_000).contains(&asked), "{asked}");

    // A classification of points is held to it as well.
    set_operation_work(Some(1_000_000));
    let points: Vec<Vec3> = (0..200)
        .map(|k| Vec3::new(15.0, 4.5 * f64::from(k), 10.0))
        .collect();
    let placed = classify_points(&long, &points, &tol);
    let text = placed
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    println!("200 points in the long block: {text}");
    assert!(text.contains(TOO_MUCH), "{placed:?}");

    // Hostile bounds: none, or of one unit. Refused or built, never a
    // panic; an operation without freeform faces asks for nothing.
    for units in [1, 2, 1000] {
        set_operation_work(Some(units));
        let made = boolean(&long, &tool, BooleanOp::Union, &tol);
        assert!(
            made.as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains(TOO_MUCH)),
            "{units}: {:?}",
            made.as_ref().err()
        );
        let a = primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
        let b = primitives::block(
            &Frame::WORLD.at(Vec3::new(1.0, 1.0, 1.0)).unwrap(),
            2.0,
            2.0,
            2.0,
        )
        .unwrap();
        let plain = boolean(&a, &b, BooleanOp::Union, &tol);
        assert!(
            plain.is_ok()
                || plain
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.to_string().contains(TOO_MUCH)),
            "{units}: {:?}",
            plain.err()
        );
        let inside = classify_points(&a, &[Vec3::new(1.0, 1.0, 1.0)], &tol);
        assert!(
            inside.as_ref().is_ok_and(|p| p == &[Place::Inside])
                || inside
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.to_string().contains(TOO_MUCH)),
            "{units}: {inside:?}"
        );
    }
    set_operation_work(None);

    // At the real bound the plain blocks are built as ever.
    let a = primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
    let b = primitives::block(
        &Frame::WORLD.at(Vec3::new(1.0, 1.0, 1.0)).unwrap(),
        2.0,
        2.0,
        2.0,
    )
    .unwrap();
    assert!(boolean(&a, &b, BooleanOp::Union, &tol).is_ok());
}
