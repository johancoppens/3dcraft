//! The offset of profiles at the sizes sketch patterns make: perforated
//! sheets of thousands of holes offset by a kerf keep their closed-form
//! area, and the time an offset takes grows near linearly with its pieces.
//!
//! `offset_timings` (ignored) prints offset times per size and layout; run
//! it in release with
//! `cargo test --release -p craft3d-kernel --test offset_scale -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{circle, polygon, rectangle};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::offset_profile::{Corners, MAX_PIECES, offset_profiles};
use craft3d_kernel::profile::{Profile, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;
use std::time::{Duration, Instant};

/// A `cols × rows` sheet of round holes of radius `r` at pitch `pitch`,
/// with a margin of one pitch: the outline first (ids 0..4), then the
/// holes. Its width and height.
fn sheet(cols: usize, rows: usize, pitch: f64, r: f64) -> (Vec<SketchCurve>, f64, f64) {
    let (w, h) = ((cols + 1) as f64 * pitch, (rows + 1) as f64 * pitch);
    let mut curves = rectangle(0, 1, (0.0, w), (0.0, h));
    for i in 0..cols * rows {
        let c = [(i % cols + 1) as f64 * pitch, (i / cols + 1) as f64 * pitch];
        curves.push(circle(4 + i as u64, c, r));
    }
    (curves, w, h)
}

/// The area of the sheet offset by `d` with round corners: the outline
/// grows by `d` with arcs of radius `d` at its four corners, and every
/// hole's radius shrinks by `d`.
fn sheet_area(w: f64, h: f64, holes: usize, r: f64, d: f64) -> f64 {
    w * h + 2.0 * d * (w + h) + PI * d * d - holes as f64 * PI * (r - d) * (r - d)
}

/// A strip `n p` long and `height` high whose top is a saw of `n` teeth
/// of pitch `p` and depth `depth`: `2 n + 3` lines.
fn saw(n: usize, p: f64, height: f64, depth: f64) -> Vec<SketchCurve> {
    let l = n as f64 * p;
    let mut points = vec![[0.0, 0.0], [l, 0.0], [l, height]];
    for k in (0..n).rev() {
        points.push([(k as f64 + 0.5) * p, height + depth]);
        points.push([k as f64 * p, height]);
    }
    // The last valley is the corner above the start.
    polygon(0, 1, &points)
}

/// A star of `n` rays from radius 1 to radius 1000: `2 n` lines. Offset
/// inward by more than 1 its rays part, and the offsets of the rays cross
/// near the middle, nearly every pair of them.
fn star(n: usize) -> Vec<SketchCurve> {
    let point = |k: usize| {
        let a = PI * k as f64 / n as f64;
        let r = if k.is_multiple_of(2) { 1.0 } else { 1000.0 };
        [r * a.cos(), r * a.sin()]
    };
    let points: Vec<[f64; 2]> = (0..2 * n).map(point).collect();
    polygon(0, 1, &points)
}

fn profiles(curves: &[SketchCurve], t: &Tolerance) -> Vec<Profile> {
    detect_profiles(curves, t).unwrap_or_else(|e| panic!("{e}"))
}

/// One offset, timed: the pieces it has, or the error.
fn timed(input: &[Profile], d: f64, corners: Corners, t: &Tolerance) -> (String, Duration) {
    let start = Instant::now();
    let result = offset_profiles(input, d, corners, t);
    let took = start.elapsed();
    let what = match result {
        Ok(o) => format!(
            "{} pieces, {} loops{}",
            o.curves.len(),
            o.loops_after,
            if o.is_whole() { "" } else { ", not whole" }
        ),
        Err(e @ KernelError::TooManyOffsetCurves { .. }) => format!("refused: {e}"),
        Err(e) => format!("error: {e}"),
    };
    (what, took)
}

#[test]
#[ignore = "timings; run in release with --ignored --nocapture"]
fn offset_timings() {
    let t = Tolerance::default();
    type Layout = (&'static str, f64, Corners, fn(usize) -> Vec<SketchCurve>);
    let layouts: [Layout; 6] = [
        (
            "star whose rays part (crossings)",
            -2.0,
            Corners::Round,
            |n| star(n / 3),
        ),
        (
            "sheet of round holes, kerf outward",
            0.1,
            Corners::Round,
            |n| {
                let side = (n as f64).sqrt().ceil() as usize;
                sheet(side, n.div_ceil(side), 10.0, 3.0).0
            },
        ),
        ("sheet of round holes, inward", -0.1, Corners::Round, |n| {
            let side = (n as f64).sqrt().ceil() as usize;
            sheet(side, n.div_ceil(side), 10.0, 3.0).0
        }),
        (
            "sheet whose holes grow into each other (bisection)",
            -2.5,
            Corners::Round,
            |n| {
                let side = (n as f64).sqrt().ceil() as usize;
                sheet(side, n.div_ceil(side), 10.0, 3.0).0
            },
        ),
        ("saw-tooth strip, round outward", 0.2, Corners::Round, |n| {
            saw(n / 3, 2.0, 10.0, 1.0)
        }),
        ("saw-tooth strip, sharp inward", -0.2, Corners::Sharp, |n| {
            saw(n / 2, 2.0, 10.0, 1.0)
        }),
    ];
    // `CRAFT3D_OFFSET_LAYOUT` picks the layouts whose name holds it.
    let only = std::env::var("CRAFT3D_OFFSET_LAYOUT").unwrap_or_default();
    for (name, d, corners, make) in layouts {
        if !name.contains(only.as_str()) {
            continue;
        }
        for n in [1_000, 4_000, 16_000, 64_000, 250_000] {
            let curves = make(n);
            let input = profiles(&curves, &t);
            let (what, took) = timed(&input, d, corners, &t);
            println!("{name}: {} curves in: {what}: {took:.2?}", curves.len());
            if took.as_secs() >= 10 {
                break;
            }
        }
    }
}

/// The sheet offset by `d` extrudes by `thick` to the closed-form volume.
fn extruded_sheet(cols: usize, k: f64, d: f64, thick: f64) {
    let (pitch, r) = (10.0 * k, 3.0 * k);
    let (curves, w, h) = sheet(cols, cols, pitch, r);
    let t = Tolerance {
        linear: 1e-6 * k,
        ..Tolerance::default()
    };
    let input = profiles(&curves, &t);
    let outline = offset_profiles(&input, d, Corners::Round, &t).unwrap();
    assert!(outline.is_whole());
    assert_eq!(outline.curves.len(), cols * cols + 8);
    let body = extrude(&outline.profiles, &Frame::WORLD, 0.0, thick, &t)
        .unwrap()
        .body;
    assert_eq!(check(&body, &t), vec![]);
    let volume = mass_properties(&body).unwrap().volume;
    let expected = sheet_area(w, h, cols * cols, r, d) * thick;
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "{volume} vs {expected}"
    );
}

/// The laser's beam path of a 70 × 70 perforated sheet (4900 holes and
/// its outline: 4908 pieces, more than the offset's old cap of 4096): the
/// outline offset by the kerf's half extrudes to the closed form
/// `(W H + 2 d (W + H) + π d² − 4900 π (r − d)²) t`, at three scales.
#[test]
fn a_70_by_70_sheet_offset_by_a_kerf_extrudes() {
    for k in [1e-3, 1.0, 1e3] {
        extruded_sheet(70, k, 0.075 * k, 2.0 * k);
    }
}

/// A regular `n`-gon of radius 1000.
fn ngon(n: usize) -> Vec<SketchCurve> {
    let points: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let a = 2.0 * PI * i as f64 / n as f64;
            [1000.0 * a.cos(), 1000.0 * a.sin()]
        })
        .collect();
    polygon(0, 1, &points)
}

/// An outline with more pieces than an offset takes says so, at once (no
/// search for a distance, which would not help): a polygon of 131 073
/// sides, whose round corners make 262 146 pieces, two more than
/// [`MAX_PIECES`]. With sharp corners, which add none where lines
/// meet, it fits.
#[test]
fn too_many_pieces_are_passed_on() {
    let t = Tolerance::default();
    let n = MAX_PIECES / 2 + 1;
    let input = profiles(&ngon(n), &t);
    let started = Instant::now();
    let err = offset_profiles(&input, 0.5, Corners::Round, &t).unwrap_err();
    assert!(
        matches!(err, KernelError::TooManyOffsetCurves { count, limit } if count == 2 * n && limit == MAX_PIECES),
        "{err:?}"
    );
    let refused = started.elapsed();
    let o = offset_profiles(&input, 0.5, Corners::Sharp, &t).unwrap();
    assert_eq!(o.curves.len(), n);
    println!(
        "refused in {refused:.2?}, sharp in {:.2?}",
        started.elapsed() - refused
    );
}

/// Inward past the outline's reach the search for the largest distance
/// that fits meets a bound on the work (a strip whose bottom is a saw of
/// 131 000 teeth: each tip adds an arc inward, and at a large distance
/// every arc meets every other): the reason stays the outline's, with no
/// largest distance rather than 0.
#[test]
#[ignore = "slow; run in release with --ignored"]
fn a_bound_ends_the_search_for_a_distance() {
    let t = Tolerance::default();
    let teeth = 131_000;
    let mut points: Vec<[f64; 2]> = Vec::new();
    for i in 0..teeth {
        let x = 2.0 * i as f64;
        points.push([x, 0.0]);
        points.push([x + 1.0, 1.0]);
    }
    let length = 2.0 * teeth as f64;
    points.push([length, 0.0]);
    points.push([length, 20.0]);
    points.push([0.0, 20.0]);
    let input = profiles(&polygon(1, 1, &points), &t);
    let started = Instant::now();
    let err = offset_profiles(&input, -2.0 * length, Corners::Round, &t).unwrap_err();
    assert!(
        matches!(
            &err,
            KernelError::OffsetProfile {
                reason: "leaves nothing of a region",
                largest: None,
                ..
            }
        ),
        "{err:?}"
    );
    println!("refused in {:.2?}", started.elapsed());
    // The search's first try, at half the outline's reach, is the one the
    // bound refuses.
    let reach = 0.5 * length.hypot(20.0);
    let err = offset_profiles(&input, -reach / 2.0, Corners::Round, &t).unwrap_err();
    assert!(matches!(err, KernelError::CrowdedOffset { .. }), "{err:?}");
    println!("{err}");
}
