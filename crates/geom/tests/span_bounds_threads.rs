//! Written by the review of 2026-11-17 (review-grid-scale): the bounds a
//! spline surface keeps per
//! whole span (`NurbsSurface::whole_span_bounds`, made ahead on the pool's
//! threads by `make_whole_span_bounds`), held by means other than the
//! author's: with 0, 1, 2, 3 and 7 of the pool's threads free and on
//! repeated runs, from several threads at once on a surface and its
//! clones, after a transform of a surface whose bounds were made, under
//! an exhausted bound on the work, with hostile indices; and what they
//! take in memory. Run with `--test-threads 1` (the pool is the
//! process's).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

use craft3d_geom::allowance;
use craft3d_geom::grid::{Spacing, interpolate};
use craft3d_geom::span_bounds::{SPANS_PER_THREAD, SpanBounds};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::pool::{SHARED_THREADS, lease};
use craft3d_math::{Portable, Transform, Vec3};

fn bits(b: &SpanBounds) -> [u64; 5] {
    let (a, c, d) = b.second.unwrap_or((f64::NAN, f64::NAN, f64::NAN));
    [
        a.to_bits(),
        c.to_bits(),
        d.to_bits(),
        b.travel.0.to_bits(),
        b.travel.1.to_bits(),
    ]
}

/// The surfaces: a polynomial grid sheet of 50 × 40 points, the same
/// lifted to rational with uneven weights, and a degree-5 by degree-2
/// surface with knots of several multiplicities; each with more whole
/// spans than two threads' share.
fn surfaces() -> Vec<(&'static str, NurbsSurface)> {
    let (rows, columns) = (40, 50);
    let points: Vec<Vec3> = (0..rows)
        .flat_map(|r| (0..columns).map(move |c| (r, c)))
        .map(|(r, c)| {
            let (x, y) = (c as f64, r as f64);
            Vec3::new(x, y, (0.3 * x).psin() * (0.2 * y).pcos() - 0.0)
        })
        .collect();
    let sheet = interpolate(&points, rows, columns, Spacing::Even, 1e-6, 1e-9)
        .unwrap()
        .surface;
    let (cu, cv) = sheet.counts();
    let (du, dv) = sheet.degrees();
    let weights: Vec<f64> = (0..cu * cv)
        .map(|i| 0.5 + ((i * 7919) % 13) as f64 / 6.0)
        .collect();
    let rational = NurbsSurface::new(
        du,
        dv,
        sheet.knots_u().to_vec(),
        sheet.knots_v().to_vec(),
        cu,
        cv,
        sheet.control_points().to_vec(),
        weights,
    )
    .unwrap();
    let (nu, nv) = (90, 30);
    let mut ku = vec![0.0; 6];
    let inner_u = nu - 6;
    let mut i = 0;
    while ku.len() < nu {
        let value = (i + 1) as f64 / (inner_u + 1) as f64;
        let times = 1 + i % 3;
        for _ in 0..times {
            if ku.len() < nu {
                ku.push(value);
            }
        }
        i += 1;
    }
    ku.extend([1.0; 6]);
    let mut kv = vec![-0.0; 3];
    for j in 0..nv - 3 {
        kv.push((j + 1) as f64 / (nv - 2) as f64);
    }
    kv.extend([1.0; 3]);
    let ctrl: Vec<Vec3> = (0..nu * nv)
        .map(|k| {
            let (i, j) = (k / nv, k % nv);
            Vec3::new(i as f64, j as f64, ((i * j) % 5) as f64 * 0.3 - 0.0)
        })
        .collect();
    let quintic = NurbsSurface::new(5, 2, ku, kv, nu, nv, ctrl, vec![1.0; nu * nv]).unwrap();
    vec![
        ("sheet", sheet),
        ("rational", rational),
        ("quintic", quintic),
    ]
}

/// The same surface made anew (nothing kept).
fn anew(s: &NurbsSurface) -> NurbsSurface {
    let (du, dv) = s.degrees();
    let (cu, cv) = s.counts();
    NurbsSurface::new(
        du,
        dv,
        s.knots_u().to_vec(),
        s.knots_v().to_vec(),
        cu,
        cv,
        s.control_points().to_vec(),
        s.weights().to_vec(),
    )
    .unwrap()
}

/// Each span's bounds made afresh on a surface that keeps nothing, with
/// the work each was charged.
fn fresh(s: &NurbsSurface) -> Vec<([u64; 5], u64)> {
    let spans = s.bezier_spans().unwrap();
    spans
        .iter()
        .map(|span| {
            let width = (span.u.1 - span.u.0, span.v.1 - span.v.0);
            let (b, units) = allowance::measured(|| SpanBounds::of(&span.patch, width));
            (bits(&b), units)
        })
        .collect()
}

#[test]
fn kept_bounds_with_any_number_of_threads_are_the_fresh_bounds() {
    for (name, surface) in surfaces() {
        let want = fresh(&anew(&surface));
        let count = want.len();
        assert!(count >= 2 * SPANS_PER_THREAD, "{name}: {count} spans");
        let all: Vec<usize> = (0..count).collect();
        for free in [0, 1, 2, 3, SHARED_THREADS] {
            for round in 0..2 {
                let held = lease(SHARED_THREADS - free);
                let s = anew(&surface);
                let ((), ahead) = allowance::measured(|| s.make_whole_span_bounds(&all));
                drop(held);
                assert_eq!(ahead, 0, "{name}: free {free}");
                // Asked in another order than made.
                let mut got = vec![([0_u64; 5], 0_u64); count];
                for index in (0..count).rev() {
                    let (b, units) = allowance::measured(|| s.whole_span_bounds(index).unwrap());
                    got[index] = (bits(&b), units);
                }
                assert!(got == want, "{name}: free {free}, round {round}");
                // Asked again: found kept, the same.
                for index in 0..count {
                    let (b, units) = allowance::measured(|| s.whole_span_bounds(index).unwrap());
                    assert_eq!((bits(&b), units), want[index], "{name}: {index}");
                }
            }
        }
        println!(
            "{name}: {count} spans, the same bits and work with 0, 1, 2, 3 and 7 threads free"
        );
    }
}

#[test]
fn bounds_asked_from_many_threads_at_once_are_the_fresh_bounds() {
    for (name, surface) in surfaces() {
        let want = fresh(&anew(&surface));
        let count = want.len();
        let s = anew(&surface);
        let clone_before = s.clone();
        let all: Vec<usize> = (0..count).collect();
        let results: Vec<Vec<([u64; 5], u64)>> = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for t in 0..6 {
                let (s, clone_before, all) = (&s, &clone_before, &all);
                handles.push(scope.spawn(move || {
                    let target = if t % 2 == 0 { s } else { clone_before };
                    if t < 3 {
                        target.make_whole_span_bounds(all);
                    }
                    let mut out = vec![([0_u64; 5], 0_u64); all.len()];
                    let order: Vec<usize> = if t % 3 == 0 {
                        all.clone()
                    } else {
                        all.iter().rev().copied().collect()
                    };
                    for index in order {
                        let (b, units) =
                            allowance::measured(|| target.whole_span_bounds(index).unwrap());
                        out[index] = (bits(&b), units);
                    }
                    out
                }));
            }
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (t, got) in results.iter().enumerate() {
            assert!(*got == want, "{name}: thread {t}");
        }
        let clone_after = s.clone();
        for index in 0..count {
            let (b, units) = allowance::measured(|| clone_after.whole_span_bounds(index).unwrap());
            assert_eq!((bits(&b), units), want[index]);
        }
        println!("{name}: six threads at once, the same bits and work");
    }
}

/// A surface made from another whose bounds were made (moved, turned) is
/// a surface of its own: its bounds are its own spans', not the first's.
#[test]
fn a_transformed_surface_does_not_find_the_bounds_of_the_first() {
    for (name, surface) in surfaces() {
        let s = anew(&surface);
        let count = s.bezier_spans().unwrap().len();
        let all: Vec<usize> = (0..count).collect();
        s.make_whole_span_bounds(&all);
        for index in 0..count {
            s.whole_span_bounds(index).unwrap();
        }
        let turn = Transform::rotation(Vec3::new(1.0, 2.0, 2.0), 0.7).unwrap();
        let stretched = Transform::translation(Vec3::new(5.0, -3.0, 1e3));
        for t in [turn, stretched] {
            let moved = match Surface::Nurbs(s.clone()).transformed(&t).unwrap() {
                Surface::Nurbs(m) => m,
                other => panic!("{other:?}"),
            };
            let want = fresh(&anew(&moved));
            for (index, w) in want.iter().enumerate() {
                let (b, units) = allowance::measured(|| moved.whole_span_bounds(index).unwrap());
                assert_eq!((bits(&b), units), *w, "{name}: span {index}");
            }
        }
        println!("{name}: a moved or turned surface has its own bounds");
    }
}

/// Made under a bound on the work that is reached, a span's bounds are
/// given (the same numbers) and not kept: asked again outside the bound
/// they are made again and charged.
#[test]
fn bounds_made_past_a_bound_are_not_kept() {
    let (name, surface) = surfaces().remove(0);
    let s = anew(&surface);
    let want = fresh(&anew(&surface));
    let got = allowance::within(0, || {
        let _ = allowance::pass();
        s.whole_span_bounds(3).unwrap()
    });
    assert_eq!(bits(&got), want[3].0, "{name}");
    let (again, units) = allowance::measured(|| s.whole_span_bounds(3).unwrap());
    assert_eq!((bits(&again), units), want[3], "{name}");
}

/// Indices past the spans, repeated or huge: nothing panics, nothing is
/// kept wrongly.
#[test]
fn hostile_indices_change_nothing() {
    let (_, surface) = surfaces().remove(0);
    let s = anew(&surface);
    let count = s.bezier_spans().unwrap().len();
    let mut hostile: Vec<usize> = vec![usize::MAX, count, count + 1];
    hostile.extend(std::iter::repeat_n(0, 3 * SPANS_PER_THREAD));
    hostile.extend((0..count).rev());
    s.make_whole_span_bounds(&hostile);
    s.make_whole_span_bounds(&[]);
    assert!(s.whole_span_bounds(usize::MAX).is_none());
    assert!(s.whole_span_bounds(count).is_none());
    let want = fresh(&anew(&surface));
    for (index, w) in want.iter().enumerate() {
        let (b, units) = allowance::measured(|| s.whole_span_bounds(index).unwrap());
        assert_eq!((bits(&b), units), *w);
    }
}

/// What the kept bounds take: a slot per span, and at the grid's cap.
#[test]
#[ignore = "prints memory; run in release"]
fn memory_of_kept_bounds() {
    let slot = std::mem::size_of::<std::sync::OnceLock<(SpanBounds, u64)>>();
    let span = std::mem::size_of::<craft3d_geom::BezierSpan>();
    println!(
        "a slot {slot} bytes; a span {span} bytes besides its 16 points of 32 bytes (bicubic)"
    );
    let peak = || {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|kib| kib.parse::<f64>().ok())
            })
            .map_or(f64::NAN, |kib| kib / 1024.0)
    };
    for (rows, columns) in [(512, 512), (256, 1024)] {
        let points: Vec<Vec3> = (0..rows)
            .flat_map(|r| (0..columns).map(move |c| (r, c)))
            .map(|(r, c)| {
                let (x, y) = (c as f64, r as f64);
                Vec3::new(x, y, (0.05 * x).psin() * (0.07 * y).pcos())
            })
            .collect();
        let s = interpolate(&points, rows, columns, Spacing::Even, 1e-6, 1e-9)
            .unwrap()
            .surface;
        drop(points);
        let before = peak();
        let t = std::time::Instant::now();
        let spans = s.bezier_spans_shared().unwrap();
        let made_spans = t.elapsed();
        let with_spans = peak();
        let all: Vec<usize> = (0..spans.len()).collect();
        let t = std::time::Instant::now();
        s.make_whole_span_bounds(&all);
        let made_bounds = t.elapsed();
        drop(all);
        let with_bounds = peak();
        println!(
            "{rows} x {columns}: {} spans in {made_spans:?} (+{:.1} MiB resident), their bounds in {made_bounds:?} (+{:.1} MiB, {:.0} bytes a span)",
            spans.len(),
            with_spans - before,
            with_bounds - with_spans,
            (with_bounds - with_spans) * 1024.0 * 1024.0 / spans.len() as f64
        );
    }
}
