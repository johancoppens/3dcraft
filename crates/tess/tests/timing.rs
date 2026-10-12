//! Triangle counts and timings per reference solid. Not a test of
//! anything: run it for the numbers, in release:
//!
//! `cargo test --release -p craft3d-tess --test timing -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Frame;
use craft3d_tess::{TessParams, tessellate_body_with_threads};
use craft3d_testkit::{solids, trimmed};
use std::time::Instant;

#[test]
#[ignore = "prints timings; run in release with --ignored --nocapture"]
fn timings_per_fixture() {
    let mut all = solids::all(&Frame::WORLD, 1.0).unwrap();
    all.extend(trimmed::all(&Frame::WORLD, 1.0).unwrap());
    all.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0, 1.0, 1.9).unwrap());
    println!(
        "{:30} {:>9} {:>10} {:>10} {:>10}",
        "solid", "tolerance", "triangles", "ms (1 thr)", "ms (auto)"
    );
    for solid in &all {
        for tolerance in [1e-3, 1e-4] {
            let params = TessParams {
                chordal_tolerance: tolerance,
                ..TessParams::default()
            };
            let time = |threads: usize| {
                // The best of three: the first run pays for page faults.
                let mut best = f64::INFINITY;
                let mut triangles = 0;
                for _ in 0..3 {
                    let start = Instant::now();
                    let mesh = tessellate_body_with_threads(&solid.body, &params, threads).unwrap();
                    best = best.min(start.elapsed().as_secs_f64() * 1e3);
                    triangles = mesh.triangles.len();
                }
                (triangles, best)
            };
            let (triangles, single) = time(1);
            let (_, auto) = time(0);
            println!(
                "{:30} {tolerance:>9.0e} {triangles:>10} {single:>10.2} {auto:>10.2}",
                solid.name
            );
        }
    }
}
