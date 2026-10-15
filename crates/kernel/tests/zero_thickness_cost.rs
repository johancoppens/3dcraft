//! What the zero-thickness refusal costs (docs/design/booleans.md): rows
//! of booleans timed with the refusal and without it, in release:
//!
//! `cargo test --release -p craft3d-kernel --test zero_thickness_cost -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{BooleanOp, BooleanOptions, boolean_with};
use craft3d_topo::{Body, TouchScope, touches};
use std::time::Instant;

fn options(refuse: bool) -> BooleanOptions {
    BooleanOptions {
        refuse_zero_thickness: refuse,
        ..BooleanOptions::default()
    }
}

/// A row of `n` unit cubes 0.5 apart along y (all at the same x), united
/// one by one; the seconds it took and the body.
fn row(n: usize, refuse: bool) -> (f64, Body) {
    let tol = tolerance(1.0);
    let started = Instant::now();
    let mut body = boxed(1.0, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    for i in 1..n {
        let y = 1.5 * i as f64;
        let cube = boxed(1.0, [0.0, y, 0.0], [1.0, y + 1.0, 1.0]);
        body = boolean_with(&body, &cube, BooleanOp::Union, &tol, &options(refuse))
            .unwrap()
            .body;
    }
    (started.elapsed().as_secs_f64(), body)
}

/// A long block with a ball cavity, then `n` holes drilled one by one
/// along it: every result has two shells.
fn drilled(n: usize, refuse: bool) -> f64 {
    let tol = tolerance(1.0);
    let length = 2.0 * n as f64 + 2.0;
    let mut body = boxed(1.0, [0.0, 0.0, 0.0], [length, 4.0, 2.0]);
    let cavity = ball(1.0, [length - 1.0, 2.0, 1.0], 0.5);
    body = boolean_with(&body, &cavity, BooleanOp::Subtract, &tol, &options(refuse))
        .unwrap()
        .body;
    let started = Instant::now();
    for i in 0..n {
        let x = 1.0 + 2.0 * i as f64;
        let drill = rod(1.0, [x, 1.0, -1.0], 0.4, 4.0);
        body = boolean_with(&body, &drill, BooleanOp::Subtract, &tol, &options(refuse))
            .unwrap()
            .body;
        assert_eq!(body.shells.len(), 2);
    }
    started.elapsed().as_secs_f64()
}

#[test]
#[ignore = "timings; run in release with --ignored --nocapture"]
fn what_the_refusal_costs() {
    for n in [250, 1000] {
        let (with, body) = row(n, true);
        let (without, _) = row(n, false);
        println!(
            "a row of {n} disjoint cubes united one by one: {with:.2} s with the refusal, \
             {without:.2} s without ({} shells)",
            body.shells.len()
        );
        let started = Instant::now();
        let found = touches(&body, 2e-6, 1, TouchScope::All).unwrap();
        println!(
            "  craft3d_topo::touches on the {}-vertex result: {:.1} ms, {} found",
            body.vertices.len(),
            started.elapsed().as_secs_f64() * 1e3,
            found.len()
        );
    }
    for n in [100, 200] {
        let with = drilled(n, true);
        let without = drilled(n, false);
        println!(
            "{n} holes drilled one by one into a block with a cavity: {with:.2} s with the \
             refusal, {without:.2} s without"
        );
    }
}
