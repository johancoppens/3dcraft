//! Booleans of blocks whose flat sides are weighted NURBS: the plane drawn
//! as a bicubic NURBS of six by six control points whose weights run from
//! 1e-3 to 1e3. Every step of a winding about a point of such a face asks
//! for the least singular value of its chart near the step, and the search
//! splits the pieces of the leaves within reach far down; until
//! 2026-11-15 the store of those pieces held 16 384 a surface, less than
//! the searches along a boundary look into, and the intersection of such
//! a block 30 × 30 × 20 with a block through three of its sides ran for
//! most of an hour, making the same pieces again and again
//! (docs/design/booleans.md › *A boolean that runs for most of an hour*).
//!
//! Each boolean is held to the closed form by means apart from the kernel
//! (its faces as polygons: flat, on the boundary of the boxes' CSG, the
//! volume by the divergence theorem, points classified by a ray's
//! crossings), runs under a deadline that ends the whole process past it
//! (so a regression cannot hang a run), and to having made no piece twice,
//! counted, not timed.
//!
//! `cargo test -p craft3d-kernel --release --test boolean_weighted_chart --
//! --include-ignored --nocapture` also runs the long blocks and the same
//! booleans with a store of the old size, each in a process of its own
//! that ends at its deadline, and prints the seconds before and after.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::{BooleanOp, boolean, count_fine_remakes, fine_work};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A block of `size` whose faces facing +x, +y and +z are flat bicubic
/// NURBS of six by six control points whose weights run from 1e-3 to 1e3
/// (seeded by `seed`), as `classify_far.rs` makes it.
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

/// Signed distance to the box `[lo, hi]` (negative inside).
fn sd_box(p: Vec3, lo: Vec3, hi: Vec3) -> f64 {
    let c = (lo + hi) * 0.5;
    let h = (hi - lo) * 0.5;
    let q = Vec3::new(
        (p.x - c.x).abs() - h.x,
        (p.y - c.y).abs() - h.y,
        (p.z - c.z).abs() - h.z,
    );
    let outside = Vec3::new(q.x.max(0.0), q.y.max(0.0), q.z.max(0.0)).length();
    outside + q.x.max(q.y).max(q.z).min(0.0)
}

/// The faces of a body whose faces are all flat and edges straight, as
/// polygons: per face, its loops' chains (each edge sampled at 16 points
/// along its own curve), its vector area turned to agree with the outward
/// normal at the middle of its surface's domain, and a point of it.
fn faces_as_polygons(body: &Body) -> Vec<(Vec<Vec<Vec3>>, Vec3, Vec3)> {
    let mut out = Vec::new();
    for face in &body.faces {
        let mut loops = Vec::new();
        let mut area = Vec3::ZERO;
        for &l in &face.loops {
            let Some(lp) = body.loop_(l) else { continue };
            let mut chain = Vec::new();
            for c in &lp.coedges {
                let Some(e) = body.edge(c.edge) else { continue };
                for k in 0..16 {
                    let s = f64::from(k) / 16.0;
                    let s = if c.reversed { 1.0 - s } else { s };
                    chain.push(e.curve.eval(e.range.0 + s * (e.range.1 - e.range.0)));
                }
            }
            for i in 0..chain.len() {
                area += chain[i].cross(chain[(i + 1) % chain.len()]) * 0.5;
            }
            loops.push(chain);
        }
        let ((u0, u1), (v0, v1)) = face.surface.domain();
        let (_, su, sv) = face.surface.derivatives(0.5 * (u0 + u1), 0.5 * (v0 + v1));
        let mut n = su.cross(sv).normalized().unwrap_or(Vec3::ZERO);
        if face.reversed {
            n = n * -1.0;
        }
        if area.dot(n) < 0.0 {
            area = area * -1.0;
        }
        let at = loops
            .first()
            .and_then(|l| l.first())
            .copied()
            .unwrap_or(Vec3::ZERO);
        out.push((loops, area, at));
    }
    out
}

/// Whether `body` is the set `sd < 0` of volume `volume` by its faces as
/// polygons, apart from the tessellator and the measure: every face's
/// surface flat over its whole domain and every point of every edge on
/// the boundary (both within the tolerance), the volume by the divergence
/// theorem within 1e-9 of it, and 600 seeded points classified by the
/// parity of a ray's crossings as the closed form says. `Err` says what
/// is not.
fn polygons_judge(
    body: &Body,
    sd: &dyn Fn(Vec3) -> f64,
    volume: f64,
    tol: f64,
) -> Result<(), String> {
    let faces = faces_as_polygons(body);
    let (mut flat, mut on, mut v) = (0.0_f64, 0.0_f64, 0.0);
    for (face, (loops, area, at)) in body.faces.iter().zip(&faces) {
        let n = area.normalized().unwrap_or(Vec3::ZERO);
        let ((u0, u1), (v0, v1)) = face.surface.domain();
        for i in 0..=16 {
            for j in 0..=16 {
                let q = face.surface.eval(
                    u0 + (u1 - u0) * f64::from(i) / 16.0,
                    v0 + (v1 - v0) * f64::from(j) / 16.0,
                );
                flat = flat.max((q - *at).dot(n).abs());
            }
        }
        for chain in loops {
            for &q in chain {
                on = on.max(sd(q).abs());
                flat = flat.max((q - *at).dot(n).abs());
            }
        }
        v += area.dot(*at) / 3.0;
    }
    let d = Vec3::new(0.31, 0.53, 0.79).normalized().unwrap();
    let mut state = 777_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let (lo, hi) = body.vertices.iter().fold(
        (
            Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            -Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        ),
        |(lo, hi), w| (lo.min(w.point), hi.max(w.point)),
    );
    let margin = Vec3::new(2.0, 2.0, 2.0);
    let (lo, span) = (lo - margin, hi - lo + margin * 2.0);
    let (mut agreed, mut wrong) = (0, 0);
    for _ in 0..600 {
        let p = Vec3::new(
            lo.x + span.x * next(),
            lo.y + span.y * next(),
            lo.z + span.z * next(),
        );
        let s = sd(p);
        if s.abs() < 1e-3 {
            continue;
        }
        let mut crossings = 0;
        let mut grazing = false;
        for (loops, area, at) in &faces {
            let Some(n) = area.normalized() else { continue };
            let dn = d.dot(n);
            if dn.abs() < 1e-12 {
                continue;
            }
            let t = (*at - p).dot(n) / dn;
            if t <= 0.0 {
                continue;
            }
            let x = p + d * t;
            let (a, b) = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() {
                (1, 2)
            } else if n.y.abs() >= n.z.abs() {
                (0, 2)
            } else {
                (0, 1)
            };
            let c = |q: Vec3, k: usize| [q.x, q.y, q.z][k];
            let mut inside = false;
            for chain in loops {
                for i in 0..chain.len() {
                    let (q, r) = (chain[i], chain[(i + 1) % chain.len()]);
                    let (qa, qb, ra, rb) = (c(q, a), c(q, b), c(r, a), c(r, b));
                    let (xa, xb) = (c(x, a), c(x, b));
                    if (qb > xb) != (rb > xb) {
                        let cross = qa + (xb - qb) / (rb - qb) * (ra - qa);
                        if (cross - xa).abs() < 1e-9 {
                            grazing = true;
                        }
                        if cross > xa {
                            inside = !inside;
                        }
                    }
                }
            }
            if inside {
                crossings += 1;
            }
        }
        if grazing {
            continue;
        }
        if (crossings % 2 == 1) == (s < 0.0) {
            agreed += 1;
        } else {
            wrong += 1;
        }
    }
    let off = (v - volume).abs() / volume;
    if flat <= tol && on <= tol && off <= 1e-9 && wrong == 0 && agreed > 300 {
        Ok(())
    } else {
        Err(format!(
            "{} faces flat within {flat:e}, edges up to {on:e} off the boundary, volume {v} for {volume} ({off:e} off), ray {agreed} agree {wrong} wrong",
            faces.len()
        ))
    }
}

/// Runs `work`, ending the whole process with a message past `seconds`:
/// a boolean cannot be stopped from outside, and a run that hung would
/// hold its machine for the run's whole limit.
fn within_deadline<T>(seconds: u64, what: &str, work: impl FnOnce() -> T) -> T {
    let done = Arc::new(AtomicBool::new(false));
    let watch = {
        let done = Arc::clone(&done);
        let what = what.to_string();
        std::thread::spawn(move || {
            let started = Instant::now();
            while !done.load(Ordering::Relaxed) {
                if started.elapsed() > Duration::from_secs(seconds) {
                    println!("{what}: ran past {seconds} s; ending the process");
                    std::process::exit(3);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let made = work();
    done.store(true, Ordering::Relaxed);
    let _ = watch.join();
    made
}

/// The block with weighted sides `size`, seed `seed`, intersected with the
/// block through three of its sides, `size` × (0.9, 0.85, 0.8) from
/// (0.4, 0.35, 0.45) × `size`: what it gave, the seconds, and the pieces
/// this thread made, let go of and made again.
fn intersect(size: Vec3, seed: u64, deadline: u64) -> (Result<Body, String>, f64, (u64, u64, u64)) {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let body = block_with_weighted_sides(size, seed);
    let defects = check(&body, &tol);
    assert!(defects.is_empty(), "seed {seed}: {defects:?}");
    let (at, tool) = tool_of(size);
    let tool = primitives::block(&Frame::WORLD.at(at).unwrap(), tool.x, tool.y, tool.z).unwrap();
    count_fine_remakes(true);
    let before = fine_work();
    let started = Instant::now();
    let what = format!("the intersection of the block {size:?} of seed {seed}");
    let made = within_deadline(deadline, &what, || {
        boolean(&body, &tool, BooleanOp::Intersect, &tol)
    });
    let took = started.elapsed().as_secs_f64();
    let after = fine_work();
    count_fine_remakes(false);
    (
        made.map(|m| m.body).map_err(|e| e.to_string()),
        took,
        (
            after.made - before.made,
            after.let_go - before.let_go,
            after.remade - before.remade,
        ),
    )
}

/// Where the tool of [`intersect`] starts and its size.
fn tool_of(size: Vec3) -> (Vec3, Vec3) {
    (
        Vec3::new(0.4 * size.x, 0.35 * size.y, 0.45 * size.z),
        Vec3::new(0.9 * size.x, 0.85 * size.y, 0.8 * size.z),
    )
}

/// The closed form of [`intersect`]: the boxes' common part.
fn shared(size: Vec3) -> (Vec3, Vec3) {
    let (at, tool) = tool_of(size);
    (at, size.min(at + tool))
}

/// Holds what [`intersect`] gave for `seed` to the closed form, or to a
/// refusal where `refused` says one is what the kernel answers.
fn hold(size: Vec3, seed: u64, refused: Option<&str>, deadline: u64) -> f64 {
    let (made, took, (made_pieces, let_go, remade)) = intersect(size, seed, deadline);
    println!(
        "block {size:?} seed {seed}: {} in {took:.2} s; fine pieces made {made_pieces}, let go {let_go}, made again {remade}",
        match &made {
            Ok(_) => "built".to_string(),
            Err(e) => format!("refused ({e})"),
        }
    );
    match (made, refused) {
        (Ok(made), None) => {
            let (lo, hi) = shared(size);
            let box_sd = move |p: Vec3| {
                sd_box(p, Vec3::ZERO, size).max(sd_box(
                    p,
                    tool_of(size).0,
                    tool_of(size).0 + tool_of(size).1,
                ))
            };
            let volume = (hi.x - lo.x) * (hi.y - lo.y) * (hi.z - lo.z);
            if let Err(why) = polygons_judge(&made, &box_sd, volume, 1e-6) {
                panic!("block {size:?} seed {seed}: not the closed form: {why}");
            }
        }
        (Err(e), Some(text)) => assert!(e.contains(text), "seed {seed}: {e}"),
        (made, want) => panic!(
            "seed {seed}: {} where {want:?} was expected",
            if made.is_ok() { "built" } else { "refused" }
        ),
    }
    assert_eq!(remade, 0, "seed {seed}: pieces made again");
    took
}

const FOLDS: &str = "a freeform face whose parametrisation stalls or folds somewhere";

/// The square block's seeds: 1, 3 and 4 are built and are the closed
/// form (3 and 4 ran for most of an hour with the old store), 2 and 5
/// refused at once as before; no piece is made twice.
#[test]
fn the_intersections_of_a_square_weighted_block_end_and_are_right() {
    let size = Vec3::new(30.0, 30.0, 20.0);
    for (seed, refused) in [
        (1, None),
        (2, Some(FOLDS)),
        (3, None),
        (4, None),
        (5, Some(FOLDS)),
    ] {
        hold(size, seed, refused, 300);
    }
}

/// The long block's seeds 1 and 3, built and the closed form, and the
/// square block's and the long block's with a store of the old size, each
/// in a process of its own that ends at its deadline: the seconds before
/// and after. By hand, in release (see the top of this file).
#[test]
#[ignore = "minutes: the long blocks, and the old store in processes of their own"]
fn the_long_weighted_blocks_and_the_old_store() {
    let long = Vec3::new(30.0, 900.0, 20.0);
    let square = Vec3::new(30.0, 30.0, 20.0);
    let mut after = Vec::new();
    for seed in [1, 3] {
        after.push((long, seed, hold(long, seed, None, 600)));
    }
    let deadline = 120;
    for (size, seed) in [(square, 1), (square, 3), (square, 4), (long, 1), (long, 3)] {
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args([
                "the_old_store_alone",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("WEIGHTED_OLD", format!("{} {seed} {deadline}", size.y))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text
            .lines()
            .find(|l| l.contains("old store:") || l.contains("ran past"))
            .unwrap_or("no answer");
        println!("before: block {size:?} seed {seed}: {line}");
    }
    for (size, seed, took) in after {
        println!("after: block {size:?} seed {seed}: {took:.2} s");
    }
}

/// One boolean of [`the_long_weighted_blocks_and_the_old_store`] with the
/// store of the old size (16 384 pieces a surface), in a process of its
/// own: nothing unless `WEIGHTED_OLD` says which.
#[test]
#[ignore = "run by the_long_weighted_blocks_and_the_old_store"]
fn the_old_store_alone() {
    let Ok(asked) = std::env::var("WEIGHTED_OLD") else {
        return;
    };
    let mut words = asked.split_whitespace().map(|w| w.parse::<f64>().unwrap());
    let (length, seed, deadline) = (
        words.next().unwrap(),
        words.next().unwrap() as u64,
        words.next().unwrap() as u64,
    );
    craft3d_kernel::boolean::set_fine_pieces(Some(1 << 14));
    let (made, took, (made_pieces, let_go, remade)) =
        intersect(Vec3::new(30.0, length, 20.0), seed, deadline);
    println!(
        "old store: {} in {took:.2} s; fine pieces made {made_pieces}, let go {let_go}, made again {remade}",
        if made.is_ok() { "built" } else { "refused" }
    );
}
