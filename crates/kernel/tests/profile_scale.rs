//! Profile detection at the sizes sketch patterns make: perforated sheets
//! of thousands of holes extrude with their closed-form volume, and the
//! time detection takes grows near linearly with the number of curves.
//!
//! `profile_detection_timings` (ignored) prints detection times per size
//! and layout; run it in release with
//! `cargo test --release -p craft3d-kernel --test profile_scale -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::{arc, circle, line, rectangle};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::profile::{MAX_CURVES, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;
use std::time::{Duration, Instant};

/// A `cols × rows` sheet of round holes of radius `r` at pitch `pitch`,
/// with a margin of one pitch: the outline first (ids 0..4), then the
/// holes. Area `W H − n π r²`.
fn sheet(cols: usize, rows: usize, pitch: f64, r: f64) -> (Vec<SketchCurve>, f64) {
    let (w, h) = ((cols + 1) as f64 * pitch, (rows + 1) as f64 * pitch);
    let mut curves = rectangle(0, 1, (0.0, w), (0.0, h));
    for i in 0..cols * rows {
        let c = [(i % cols + 1) as f64 * pitch, (i / cols + 1) as f64 * pitch];
        curves.push(circle(4 + i as u64, c, r));
    }
    let n = (cols * rows) as f64;
    (curves, w * h - n * PI * r * r)
}

/// A `cols × rows` sheet of slots (two lines and two half circles each,
/// length `l` between the centres, radius `r`): `4 n + 4` curves. Area
/// `W H − n (2 r l + π r²)`.
fn slotted(cols: usize, rows: usize, pitch: f64, l: f64, r: f64) -> (Vec<SketchCurve>, f64) {
    let (w, h) = ((cols + 1) as f64 * pitch, (rows + 1) as f64 * pitch);
    let mut curves = rectangle(0, 1, (0.0, w), (0.0, h));
    for i in 0..cols * rows {
        let (x, y) = ((i % cols + 1) as f64 * pitch, (i / cols + 1) as f64 * pitch);
        let (a, b) = ([x - l / 2.0, y], [x + l / 2.0, y]);
        let id = 4 + 4 * i as u64;
        curves.push(line(id, [a[0], y - r], [b[0], y - r]));
        curves.push(arc(id + 1, b, [b[0], y - r], [b[0], y + r]));
        curves.push(line(id + 2, [b[0], y + r], [a[0], y + r]));
        curves.push(arc(id + 3, a, [a[0], y + r], [a[0], y - r]));
    }
    let n = (cols * rows) as f64;
    (curves, w * h - n * (2.0 * r * l + PI * r * r))
}

/// A tall strip with `n` square holes in one column: every end of a
/// vertical side shares its x with a quarter of all ends.
fn column(n: usize) -> (Vec<SketchCurve>, f64) {
    let mut curves = rectangle(0, 1, (0.0, 3.0), (0.0, 2.0 * n as f64 + 1.0));
    for i in 0..n {
        let y = 2.0 * i as f64 + 1.0;
        curves.extend(rectangle(4 + 4 * i as u64, 1, (1.0, 2.0), (y, y + 1.0)));
    }
    (curves, 3.0 * (2.0 * n as f64 + 1.0) - n as f64)
}

/// A regular `n`-gon of radius 1000.
fn ngon(n: usize) -> (Vec<SketchCurve>, f64) {
    let p = |i: usize| {
        let a = 2.0 * PI * (i % n) as f64 / n as f64;
        [1000.0 * a.pcos(), 1000.0 * a.psin()]
    };
    let curves = (0..n).map(|i| line(i as u64, p(i), p(i + 1))).collect();
    let area = n as f64 * 1e6 * (2.0 * PI / n as f64).psin() / 2.0;
    (curves, area)
}

fn detect_timed(curves: &[SketchCurve], area: f64) -> Duration {
    let tol = Tolerance::default();
    let start = Instant::now();
    let profiles = detect_profiles(curves, &tol).unwrap_or_else(|e| panic!("{e}"));
    let took = start.elapsed();
    let total: f64 = profiles.iter().map(|p| p.area()).sum();
    assert!((total - area).abs() <= 1e-9 * area, "{total} vs {area}");
    took
}

#[test]
#[ignore = "timings; run in release with --ignored --nocapture"]
fn profile_detection_timings() {
    type Family = (&'static str, fn(usize) -> (Vec<SketchCurve>, f64));
    let families: [Family; 4] = [
        ("sheet of round holes", |n| {
            let side = (n as f64).sqrt().ceil() as usize;
            sheet(side, n.div_ceil(side), 10.0, 3.0)
        }),
        ("sheet of slots", |n| {
            let holes = n / 4;
            let side = (holes as f64).sqrt().ceil() as usize;
            slotted(side, holes.div_ceil(side), 10.0, 4.0, 2.0)
        }),
        ("square holes in one column", |n| column(n / 4)),
        ("one polygon", ngon),
    ];
    for (name, make) in families {
        for n in [1_000, 4_000, 16_000, 64_000, 100_000, 250_000, 1_000_000] {
            let (curves, area) = make(n);
            if curves.len() > MAX_CURVES {
                println!("{name}: {} curves: over the cap", curves.len());
                break;
            }
            let took = detect_timed(&curves, area);
            println!("{name}: {} curves: {took:.2?}", curves.len());
            if took.as_secs() >= 20 {
                break;
            }
        }
    }
}

/// Extrudes `curves` by `t` and checks the body and its volume.
fn extrude_checked(name: &str, curves: &[SketchCurve], area: f64, t: f64, tol: &Tolerance) {
    let at = Instant::now();
    let profiles = detect_profiles(curves, tol).unwrap_or_else(|e| panic!("{name}: {e}"));
    let detect = at.elapsed();
    let total: f64 = profiles.iter().map(|p| p.area()).sum();
    assert!(
        (total - area).abs() <= 1e-9 * area,
        "{name}: {total} vs {area}"
    );
    let at = Instant::now();
    let body = extrude(&profiles, &Frame::WORLD, 0.0, t, tol)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .body;
    let extruded = at.elapsed();
    let at = Instant::now();
    assert_eq!(check(&body, tol), vec![], "{name}");
    let checked = at.elapsed();
    let at = Instant::now();
    let volume = mass_properties(&body).unwrap().volume;
    let measured = at.elapsed();
    let expected = area * t;
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "{name}: volume {volume} vs {expected}"
    );
    println!(
        "{name}: {} curves, detect {detect:.2?}, extrude {extruded:.2?}, check {checked:.2?}, volume {measured:.2?}",
        curves.len()
    );
}

/// The roadmap's case (M5): a 70 × 70 perforated sheet, 4900 holes and
/// its outline, more than the old cap of 4096 curves, at three scales.
/// Its volume is the closed form `(W H − 4900 π r²) t`.
#[test]
fn a_perforated_sheet_of_70_by_70_holes_extrudes() {
    for k in [1e-3, 1.0, 1e3] {
        let (curves, area) = sheet(70, 70, 10.0 * k, 3.0 * k);
        assert_eq!(curves.len(), 4904);
        let tol = Tolerance {
            linear: 1e-6 * k,
            ..Tolerance::default()
        };
        extrude_checked(
            &format!("70 × 70 holes at {k}"),
            &curves,
            area,
            2.0 * k,
            &tol,
        );
    }
}

/// The weld table's plate (finding W4 of the weld table study): 10' by
/// 5' with round holes and slots, 4675 curves in the study; here 59 × 58
/// places at a pitch of 50 mm, every third row slots: 2301 holes and 1121
/// slots, 6789 curves.
#[test]
fn a_weld_table_plate_of_holes_and_slots_extrudes() {
    let (cols, rows, pitch, r) = (59, 58, 50.0, 14.0);
    let (w, h) = ((cols + 1) as f64 * pitch, (rows + 1) as f64 * pitch);
    let (mut curves, _) = slotted(0, 0, pitch, 0.0, r);
    let (mut holes, mut slots) = (0, 0);
    let mut id = 4;
    for row in 0..rows {
        let y = (row + 1) as f64 * pitch;
        for col in 0..cols {
            let x = (col + 1) as f64 * pitch;
            if row % 3 == 2 {
                // A slot across the place of a hole.
                let (l, sr) = (20.0, 9.0);
                let (a, b) = ([x - l / 2.0, y], [x + l / 2.0, y]);
                curves.push(line(id, [a[0], y - sr], [b[0], y - sr]));
                curves.push(arc(id + 1, b, [b[0], y - sr], [b[0], y + sr]));
                curves.push(line(id + 2, [b[0], y + sr], [a[0], y + sr]));
                curves.push(arc(id + 3, a, [a[0], y + sr], [a[0], y - sr]));
                id += 4;
                slots += 1;
            } else {
                curves.push(circle(id, [x, y], r));
                id += 1;
                holes += 1;
            }
        }
    }
    // The outline is the one `slotted` made for no holes: resize it.
    curves.splice(0..4, rectangle(0, 1, (0.0, w), (0.0, h)));
    let area =
        w * h - holes as f64 * PI * r * r - slots as f64 * (2.0 * 9.0 * 20.0 + PI * 9.0 * 9.0);
    assert!(curves.len() > 4096, "{}", curves.len());
    extrude_checked(
        "weld table plate",
        &curves,
        area,
        16.0,
        &Tolerance::default(),
    );
}

/// A closed cubic spline of `m` control points about `c`, wavy (radius `r`
/// and `0.9 r` in turn).
fn wavy_spline(id: u64, c: [f64; 2], r: f64, m: usize) -> SketchCurve {
    let mut control: Vec<[f64; 2]> = (0..m)
        .map(|i| {
            let a = 2.0 * PI * i as f64 / m as f64;
            let s = if i % 2 == 0 { r } else { 0.9 * r };
            [c[0] + s * a.pcos(), c[1] + s * a.psin()]
        })
        .collect();
    control.push(control[0]);
    let n = control.len();
    let mut knots = vec![0.0; 4];
    knots.extend((1..n - 3).map(|k| k as f64));
    knots.extend([(n - 3) as f64; 4]);
    SketchCurve {
        id,
        curve: craft3d_kernel::profile::Curve2::Spline(craft3d_kernel::profile::Spline2 {
            degree: 3,
            knots,
            control,
        }),
    }
}

/// Time of detection per control point of splines, by the size of each.
#[test]
#[ignore = "timings; run in release with --ignored --nocapture"]
fn spline_work_timings() {
    for m in [8, 32, 128, 512, 4095] {
        for total in [16_384, 65_536, 262_144] {
            let count = (total / (m + 1)).max(1);
            let side = (count as f64).sqrt().ceil() as usize;
            let curves: Vec<SketchCurve> = (0..count)
                .map(|k| {
                    let c = [(k % side) as f64 * 30.0, (k / side) as f64 * 30.0];
                    wavy_spline(k as u64, c, 10.0, m)
                })
                .collect();
            let started = Instant::now();
            let result = detect_profiles(&curves, &Tolerance::default());
            println!(
                "{count} splines of {m} points ({} in all): {:.2?} ({})",
                count * (m + 1),
                started.elapsed(),
                match &result {
                    Ok(p) => format!("{} profiles", p.len()),
                    Err(e) => format!("{e}"),
                }
            );
        }
    }
}

/// Splines whose control points in all pass the bound are refused at
/// once, naming the largest. (Up to the bound they are accepted; the
/// largest such input takes seconds even in release, so it is timed by
/// `spline_work_timings` and not here.)
#[test]
fn splines_with_too_many_points_in_all_are_refused() {
    use craft3d_kernel::KernelError;
    use craft3d_kernel::profile::MAX_SPLINE_POINTS_IN_ALL;
    // 65 splines of 4096 control points (the last on the first): one
    // more than the bound holds.
    let mut many: Vec<SketchCurve> = (0..65)
        .map(|k| {
            let c = [(k % 8) as f64 * 30.0, (k / 8) as f64 * 30.0];
            wavy_spline(k as u64, c, 10.0, 4095)
        })
        .collect();
    // One small spline among them, not named.
    many.push(wavy_spline(1000, [500.0, 500.0], 10.0, 8));
    // On the work of this thread, which a busy machine does not stretch.
    let started = craft3d_testkit::cost::WorkClock::start();
    let err = detect_profiles(&many, &Tolerance::default()).unwrap_err();
    assert!(started.elapsed().as_secs_f64() < 1.0);
    assert_eq!(
        err,
        KernelError::TooManySplinePoints {
            count: 65 * 4096 + 9,
            limit: MAX_SPLINE_POINTS_IN_ALL,
            curves: (0..8).collect(),
        }
    );
    // Small splines well under the bound: accepted.
    let fits: Vec<SketchCurve> = (0..64)
        .map(|k| {
            let c = [(k % 8) as f64 * 30.0, (k / 8) as f64 * 30.0];
            wavy_spline(k as u64, c, 10.0, 16)
        })
        .collect();
    assert_eq!(
        detect_profiles(&fits, &Tolerance::default()).unwrap().len(),
        64
    );
}
