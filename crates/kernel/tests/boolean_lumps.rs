//! Booleans of bodies of many disjoint lumps and of faces with many edges
//! (finding W2 of the welding table, docs/design/booleans.md › Many lumps):
//! an n × n grid of small prisms minus an n × n grid of posts, one post
//! into each prism; and a row of n blocks through the top of a plate with
//! n holes, each block beside its own hole.
//!
//! The broad phase is held to the kernel's work counts, which are the same
//! on any machine: per lump, the faces tested by box and the edges asked
//! where they meet a surface (each pass of the loops that do it, counted
//! in the loops) stay the same as the lumps grow. Testing every
//! face against every face (or every edge of a face for every face it
//! meets) makes them grow with the number of lumps. The counts are
//! process-wide, so the one test here that reads them is the only test of
//! this file that does any work outside `--ignored`.
//!
//! Timings, in release:
//! `cargo test --release -p craft3d-kernel --test boolean_lumps -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::work::{KernelWork, work};
use craft3d_topo::{Body, check, merge};
use std::time::Instant;

fn merged(bodies: &[Body]) -> Body {
    let all: Vec<&Body> = bodies.iter().collect();
    merge(&all).unwrap().0
}

/// One body of `n` × `n` blocks 8 × 8 × 2 on a pitch of 10 (the cells), and
/// one of `n` × `n` posts 2 × 2 × 4 standing 3 below them, each through the
/// bottom of its cell into the middle of it: the welding table's probe
/// `lumps.py`.
fn grids(k: f64, n: usize) -> (Body, Body) {
    let mut cells = Vec::new();
    let mut posts = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (10.0 * i as f64, 10.0 * j as f64);
            cells.push(boxed(k, [x, y, 0.0], [x + 8.0, y + 8.0, 2.0]));
            posts.push(boxed(k, [x + 3.0, y + 3.0, -3.0], [x + 5.0, y + 5.0, 1.0]));
        }
    }
    (merged(&cells), merged(&posts))
}

/// Every prism keeps its volume less the post's part in it, and the result
/// is one shell per prism: its 6 faces, the 4 sides of the pocket and the
/// pocket's floor.
fn cut(k: f64, n: usize) -> BooleanResult {
    let (cells, posts) = grids(k, n);
    let result = boolean(&cells, &posts, BooleanOp::Subtract, &tolerance(k)).unwrap();
    assert!(check(&result.body, &tolerance(k)).is_empty());
    assert_eq!(result.body.shells.len(), n * n);
    assert_eq!(result.body.faces.len(), n * n * 11);
    let want = (n * n) as f64 * (64.0 * 2.0 - 4.0 * 1.0) * k * k * k;
    assert_close(mass(&result.body).volume, want, "volume of the cut grid");
    result
}

/// A plate 10 n × 10 × 2 with `n` round holes of radius 2 along it (its
/// top and bottom are each one face with `n` holes), and `n` blocks
/// 2 × 2 × 2 through its top, one beside each hole: a block's sides meet
/// the plate's top, whose every edge is a candidate for every one of them
/// unless the edges are looked up by where they are.
fn plate_and_blocks(k: f64, n: usize) -> (Body, Body) {
    let tol = tolerance(k);
    let plate = boxed(k, [0.0, 0.0, 0.0], [10.0 * n as f64, 10.0, 2.0]);
    let rods: Vec<Body> = (0..n)
        .map(|i| rod(k, [10.0 * i as f64 + 6.0, 5.0, -1.0], 2.0, 4.0))
        .collect();
    let plate = boolean(&plate, &merged(&rods), BooleanOp::Subtract, &tol)
        .unwrap()
        .body;
    let blocks: Vec<Body> = (0..n)
        .map(|i| {
            let x = 10.0 * i as f64 + 1.0;
            boxed(k, [x, 4.0, 1.0], [x + 2.0, 6.0, 3.0])
        })
        .collect();
    (plate, merged(&blocks))
}

/// The plate less the blocks: `n` pockets in its top beside the holes.
fn pocket(k: f64, n: usize) -> BooleanResult {
    let (plate, blocks) = plate_and_blocks(k, n);
    let result = boolean(&plate, &blocks, BooleanOp::Subtract, &tolerance(k)).unwrap();
    assert!(check(&result.body, &tolerance(k)).is_empty());
    assert_eq!(result.body.shells.len(), 1);
    let hole = std::f64::consts::PI * 4.0 * 2.0;
    let want = n as f64 * (200.0 - hole - 4.0) * k * k * k;
    assert_close(
        mass(&result.body).volume,
        want,
        "volume of the pocketed plate",
    );
    result
}

fn spent(before: KernelWork) -> KernelWork {
    let after = work();
    KernelWork {
        booleans: after.booleans - before.booleans,
        face_pairs: after.face_pairs - before.face_pairs,
        face_candidates: after.face_candidates - before.face_candidates,
        edge_candidates: after.edge_candidates - before.edge_candidates,
        patch_trees: after.patch_trees - before.patch_trees,
        blend_sections: after.blend_sections - before.blend_sections,
    }
}

#[test]
fn many_lumps_cost_work_in_proportion_to_their_number_at_three_scales() {
    for k in SCALES {
        let mut per_lump = Vec::new();
        for n in [4, 16] {
            let before = work();
            cut(k, n);
            let w = spent(before);
            let lumps = (n * n) as f64;
            per_lump.push((
                w.face_candidates as f64 / lumps,
                w.face_pairs as f64 / lumps,
            ));
        }
        let ((small, pairs_small), (large, pairs_large)) = (per_lump[0], per_lump[1]);
        // 16 times the lumps: the same work per lump. Testing every face
        // would be 16 times as much per lump. (Every pass of the loops that
        // test faces by box is counted, whatever they iterate over, so the
        // count is never nothing: a lump is tested at least against its
        // post.)
        assert!(small > 0.0, "{k}: no faces tested by box");
        assert!(large <= 1.25 * small, "{k}: {small} then {large} per lump");
        assert_eq!(pairs_small, pairs_large, "{k}: face pairs per lump");

        let mut per_block = Vec::new();
        for n in [5, 40] {
            // (The plate's holes are a boolean too: only the pockets count.)
            let (plate, blocks) = plate_and_blocks(k, n);
            let before = work();
            let result = boolean(&plate, &blocks, BooleanOp::Subtract, &tolerance(k)).unwrap();
            let w = spent(before);
            assert_eq!(result.body.shells.len(), 1);
            per_block.push(w.edge_candidates as f64 / n as f64);
        }
        let (small, large) = (per_block[0], per_block[1]);
        // 8 times the holes in the plate's top: the same edges asked per
        // block. Asking every edge of the top would be 8 times as many.
        assert!(small > 0.0, "{k}: no edges asked");
        assert!(large <= 1.25 * small, "{k}: {small} then {large} per block");
        pocket(k, 3);
    }
}

fn digest(result: &BooleanResult) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{result:?}").hash(&mut hasher);
    hasher.finish()
}

#[test]
#[ignore = "timings; run in release with --ignored --nocapture"]
fn how_the_cost_grows_with_the_lumps() {
    for n in [10, 20, 30, 50] {
        let started = Instant::now();
        let before = work();
        let result = cut(1.0, n);
        let w = spent(before);
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "n {n:3} lumps {:5}: {seconds:.3} s, {:.2} ms per lump, {} face pairs, \
             {} face candidates, {} edge candidates, digest {:016x}",
            n * n,
            1e3 * seconds / (n * n) as f64,
            w.face_pairs,
            w.face_candidates,
            w.edge_candidates,
            digest(&result)
        );
    }
    for n in [50, 200, 800] {
        let started = Instant::now();
        let before = work();
        let result = pocket(1.0, n);
        let w = spent(before);
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "plate with {n:4} holes and blocks: {seconds:.3} s (with the holes), {} face pairs, \
             {} face candidates, {} edge candidates, digest {:016x}",
            w.face_pairs,
            w.face_candidates,
            w.edge_candidates,
            digest(&result)
        );
    }
}
