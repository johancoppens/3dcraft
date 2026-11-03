//! Holes across the bends that were refused until 2026-11-03
//! (docs/design/sheetmetal.md › *Holes across a bend*), held to closed
//! forms at 1e-3, 1 and 1e6:
//!
//! - a slot across a jog's two bends and its web, drawn where it lies in
//!   the flat pattern (a cut-out): the volume less the slot on the flats
//!   and the web times `T` and on each strip times `T (R + T/2) / (R + K T)`,
//!   the flat area less the slot;
//! - a round hole and a slot of the base drawn across a jog's line (as if
//!   the bends had no width): the near piece where it is, the far piece
//!   carried with the far side, joined across the strips and the web, so
//!   the flat area loses the disc and `10 (2 BA + w)`, the volume the disc
//!   and the web times `T` and the strips' share.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, JogSpec, Part, Spec, build, flat_area, web_length,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn lines(points: &[[f64; 2]], first: u64, k: f64) -> Vec<SketchCurve> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            SketchCurve {
                id: first + i as u64,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect()
}

fn profile(points: &[[f64; 2]], first: u64, k: f64) -> Profile {
    detect_profiles(&lines(points, first, k), &tolerance(k))
        .unwrap()
        .remove(0)
}

fn slot(x0: f64, x1: f64, y0: f64, y1: f64, k: f64) -> Cutout {
    Cutout {
        profile: profile(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], 20, k),
        from: "the test".to_string(),
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} against {expected} (off by {:e}, allowed {slack:e})",
        actual - expected
    );
}

fn volume(part: &Part) -> f64 {
    mass_properties(&part.solid.body).unwrap().volume
}

#[test]
fn a_slot_across_a_jog() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.33, 0.5] {
            let (t, r, h, alpha) = (1.0 * k, 1.0 * k, 8.0 * k, PI / 3.0);
            let w = web_length(h, alpha, r, t).unwrap();
            let ba = alpha * (r + kf * t);
            let far = 50.0 * k + 2.0 * ba + w;
            let (x0, x1, y0, y1) = (45.0, far / k + 5.0, 15.0, 25.0);
            let spec = Spec {
                base: profile(
                    &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
                    1,
                    k,
                ),
                plane: Frame::WORLD,
                thickness: t,
                flanges: Vec::new(),
                relief: None,
                bend_relief: None,
                corner: Closing::OPEN,
                corners: Vec::new(),
                cutouts: vec![slot(x0, x1, y0, y1, k)],
                jogs: vec![JogSpec {
                    line: [[50.0 * k, -5.0 * k], [50.0 * k, 45.0 * k]],
                    offset: h,
                    angle: alpha,
                    direction: Direction::Up,
                    radius: r,
                    k: kf,
                    ids: [1, 2],
                }],
                ids: Vec::new(),
            };
            let part = build(&spec, &tol).unwrap_or_else(|e| panic!("K {kf} at {k}: {e}"));
            let defects = check(&part.solid.body, &tol);
            assert!(defects.is_empty(), "{defects:?}");
            let width = 10.0 * k;
            let whole = t * 40.0 * k * (100.0 * k + w) + 2.0 * alpha * 40.0 * k * t * (r + t / 2.0);
            let on_flats = width * (5.0 * k + w + 5.0 * k);
            let on_strips = width * 2.0 * ba;
            let v = whole - on_flats * t - on_strips * t * (r + t / 2.0) / (r + kf * t);
            near(volume(&part), v, 1e-8 * v, "volume");
            let a = 40.0 * k * (100.0 * k + 2.0 * ba + w) - width * (x1 - x0) * k;
            near(flat_area(&part), a, 1e-8 * a, "flat area");
        }
    }
}

fn jogged(base: Profile, cutouts: Vec<Cutout>, k: f64, kf: f64) -> Spec {
    let (t, r, h, alpha) = (1.0 * k, 1.0 * k, 8.0 * k, PI / 3.0);
    let _ = (t, r);
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: 1.0 * k,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts,
        jogs: vec![JogSpec {
            // Its middle (50, 30) clear of the holes: the middle names the flat.
            line: [[50.0 * k, -5.0 * k], [50.0 * k, 65.0 * k]],
            offset: h,
            angle: alpha,
            direction: Direction::Up,
            radius: 1.0 * k,
            k: kf,
            ids: [1, 2],
        }],
        ids: Vec::new(),
    }
}

/// The plate 100 × 40 with a hole of the base sketch: `extra` curves.
fn holed(extra: Vec<SketchCurve>, k: f64) -> Profile {
    let mut curves = lines(
        &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
        1,
        k,
    );
    curves.extend(extra);
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].holes.len(), 1);
    found.remove(0)
}

#[test]
fn a_hole_of_the_base_across_a_jogs_line() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.33, 0.5] {
            let (t, r, h, alpha) = (1.0 * k, 1.0 * k, 8.0 * k, PI / 3.0);
            let w = web_length(h, alpha, r, t).unwrap();
            let ba = alpha * (r + kf * t);
            let whole = t * 40.0 * k * (100.0 * k + w) + 2.0 * alpha * 40.0 * k * t * (r + t / 2.0);
            let share = t * (r + t / 2.0) / (r + kf * t);
            let width = 10.0 * k;
            // A disc of radius 5 about (50, 20), and a slot 10 high from
            // x = 40 to 60.
            let disc = SketchCurve {
                id: 9,
                curve: Curve2::Circle {
                    center: [50.0 * k, 20.0 * k],
                    radius: 5.0 * k,
                },
            };
            let slot = lines(
                &[[40.0, 15.0], [60.0, 15.0], [60.0, 25.0], [40.0, 25.0]],
                20,
                k,
            );
            for (extra, area) in [(vec![disc], PI * 25.0 * k * k), (slot, 200.0 * k * k)] {
                let part = build(&jogged(holed(extra, k), Vec::new(), k, kf), &tol)
                    .unwrap_or_else(|e| panic!("K {kf} at {k}: {e}"));
                let defects = check(&part.solid.body, &tol);
                assert!(defects.is_empty(), "{defects:?}");
                assert_eq!(part.cuts.len(), 1, "the hole is a cut across the jog");
                let a = 40.0 * k * (100.0 * k + 2.0 * ba + w) - area - width * (2.0 * ba + w);
                near(flat_area(&part), a, 1e-8 * a, "flat area");
                let v = whole - t * (area + width * w) - width * 2.0 * ba * share;
                near(volume(&part), v, 1e-8 * v, "volume");
            }
        }
    }
}

/// A hole of a quarter, a half and a quarter circle in a plate, plain and
/// on a jog's far side (found 2026-11-03 by the round hole across a jog's
/// line): the plate's corner where two square rims run on smoothly into
/// each other on one cylinder was solved for a point on two coinciding
/// surfaces, which drifted along their common line, and the edges went
/// round the wrong side of the circle: a body that passed the checker
/// with its volume 2 % short. The volume is the plate's less the disc.
#[test]
fn a_hole_of_arcs_of_one_circle_keeps_its_volume() {
    for k in SCALES {
        let tol = tolerance(k);
        for jog in [false, true] {
            for (cx, cy) in [(30.0, 20.0), (75.0, 20.0)] {
                let c = [cx * k, cy * k];
                let p = |deg: f64| {
                    let a = deg.to_radians();
                    [c[0] + 5.0 * k * a.cos(), c[1] + 5.0 * k * a.sin()]
                };
                let arc = |id: u64, from: f64, to: f64| SketchCurve {
                    id,
                    curve: Curve2::Arc {
                        center: c,
                        start: p(from),
                        end: p(to),
                    },
                };
                let arcs = vec![
                    arc(9, 0.0, 90.0),
                    arc(10, 90.0, 270.0),
                    arc(11, 270.0, 360.0),
                ];
                let plain_base = profile(
                    &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
                    1,
                    k,
                );
                let mut spec = jogged(holed(arcs, k), Vec::new(), k, 0.5);
                let mut plain = jogged(plain_base, Vec::new(), k, 0.5);
                if !jog {
                    spec.jogs.clear();
                    plain.jogs.clear();
                }
                let part = build(&spec, &tol).unwrap();
                let defects = check(&part.solid.body, &tol);
                assert!(defects.is_empty(), "{defects:?}");
                let v = volume(&build(&plain, &tol).unwrap()) - PI * 25.0 * k * k * k;
                near(volume(&part), v, 1e-9 * v, "the plate less the disc");
            }
        }
    }
}
