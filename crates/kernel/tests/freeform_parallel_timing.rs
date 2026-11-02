//! How long freeform booleans take side by side (rule 12): the domed
//! block of `craft3d_testkit`, each operation with a roof of its own (no
//! tree shared on the shelf), cut by a block, on one thread and on eight
//! at once, as a pattern's unions (`derive::in_order`), interference and
//! assertion sweeps run them. Prints timings only.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::Subtract;
use craft3d_math::Frame;
use craft3d_testkit::trimmed::domed_block;
use std::time::Instant;

const OPERATIONS: usize = 48;

fn operation(n: usize) {
    let height = 2.0 + 0.01 * n as f64;
    let dome = domed_block(&Frame::WORLD, 4.0, 3.0, height, n % 2 == 1)
        .unwrap()
        .body;
    let block = boxed(1.0, [0.5, 0.5, 1.0], [3.5, 2.5, 4.0]);
    let _ = solve(&dome, &block, Subtract, 1.0);
}

#[test]
#[ignore = "prints timings only"]
fn freeform_booleans_side_by_side() {
    // Warm up (the first operation pays for one-time work).
    operation(1000);
    let start = Instant::now();
    for n in 0..OPERATIONS {
        operation(n);
    }
    let one = start.elapsed().as_secs_f64();
    for (round, threads) in [3usize, 8].into_iter().enumerate() {
        let base = OPERATIONS * (round + 1);
        let start = Instant::now();
        std::thread::scope(|scope| {
            for t in 0..threads {
                scope.spawn(move || {
                    let mut n = base + t;
                    while n < base + OPERATIONS {
                        operation(n);
                        n += threads;
                    }
                });
            }
        });
        let many = start.elapsed().as_secs_f64();
        println!(
            "TIMING {OPERATIONS} freeform booleans: one thread {one:.2} s; {threads} threads {many:.2} s (x{:.2})",
            one / many
        );
    }
}
