//! How long freeform booleans take side by side (rule 12): the domed
//! block of `craft3d_testkit`, each operation with a roof of its own (no
//! tree shared on the shelf), cut by a block, on one thread and on three
//! and eight at once, as a pattern's unions (`derive::in_order`),
//! interference and assertion sweeps run them. Seven rounds; prints each
//! and the medians. Prints timings only.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::Subtract;
use craft3d_math::Frame;
use craft3d_testkit::trimmed::domed_block;
use std::time::Instant;

const OPERATIONS: usize = 48;
const ROUNDS: usize = 7;

fn operation(n: usize) {
    operation_at(2.0 + 0.001 * n as f64, n % 2 == 1);
}

fn operation_at(height: f64, nurbs: bool) {
    let dome = domed_block(&Frame::WORLD, 4.0, 3.0, height, nurbs)
        .unwrap()
        .body;
    let block = boxed(1.0, [0.5, 0.5, 1.0], [3.5, 2.5, 4.0]);
    let _ = solve(&dome, &block, Subtract, 1.0);
}

/// Seconds for `OPERATIONS` operations from `base` on, on `threads`.
fn timed(base: usize, threads: usize) -> f64 {
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
    start.elapsed().as_secs_f64()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "prints timings only"]
fn freeform_booleans_side_by_side() {
    // Warm up (the first operation pays for one-time work).
    operation_at(1.9, false);
    let mut base = 0;
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); 3];
    for round in 0..ROUNDS {
        let mut line = format!("TIMING round {round}:");
        for (i, threads) in [1usize, 3, 8].into_iter().enumerate() {
            // Roofs never asked for before: no tree from the shelf.
            let seconds = timed(base, threads);
            base += OPERATIONS;
            times[i].push(seconds);
            line.push_str(&format!(" {threads} threads {seconds:.2} s;"));
        }
        println!("{line}");
    }
    let [one, three, eight] = [0, 1, 2].map(|i| median(times[i].clone()));
    println!(
        "TIMING median of {ROUNDS}, {OPERATIONS} freeform booleans: 1 thread {one:.2} s, 3 threads {three:.2} s (x{:.2}), 8 threads {eight:.2} s (x{:.2})",
        one / three,
        one / eight
    );
}
