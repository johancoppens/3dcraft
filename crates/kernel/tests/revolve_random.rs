//! A seeded run of random profiles of lines and arcs about random axes
//! through random sweeps: every result is a valid body with its Pappus
//! volume and area, or a clean error of an expected kind; never a panic,
//! never an invalid body.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod revolve_support;

use common::{P, Rng, arc, line};
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{check, mass_properties};
use revolve_support::{Axis, clear_axis, half_disc, lathe, measure, tolerance};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Whether the curves, sampled at 256 points each, have points more than
/// `margin` to either side of the axis.
fn straddles(curves: &[SketchCurve], axis: Axis, margin: f64) -> bool {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in curves {
        for i in 0..=256 {
            let t = f64::from(i) / 256.0;
            let p = match c.curve {
                Curve2::Line { start, end } => [
                    start[0] + t * (end[0] - start[0]),
                    start[1] + t * (end[1] - start[1]),
                ],
                Curve2::Arc { center, start, end } => {
                    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                    let begin = (start[1] - center[1]).atan2(start[0] - center[0]);
                    let finish = (end[1] - center[1]).atan2(end[0] - center[0]);
                    let a = begin + t * (finish - begin).rem_euclid(TAU);
                    [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
                }
                _ => panic!("blobs are lines and arcs"),
            };
            let left = axis.left(p);
            (low, high) = (low.min(left), high.max(left));
        }
    }
    low < -margin && high > margin
}

fn between(rng: &mut Rng, low: f64, high: f64) -> f64 {
    low + (high - low) * rng.unit()
}

/// A convex polygon around `centre` whose sides are lines or arcs that
/// bulge out or in. Returns the curves and the polygon's corners.
fn blob(rng: &mut Rng, centre: P, size: f64) -> (Vec<SketchCurve>, Vec<P>) {
    let n = 3 + rng.below(6) as usize;
    // Corners at increasing angles, none too close to the next.
    let mut angles: Vec<f64> = (0..n)
        .map(|i| (i as f64 + between(rng, 0.15, 0.85)) * TAU / n as f64)
        .collect();
    let turn = between(rng, 0.0, TAU);
    for a in &mut angles {
        *a += turn;
    }
    let corners: Vec<P> = angles
        .iter()
        .map(|a| {
            let r = size * between(rng, 0.6, 1.0);
            [centre[0] + r * a.cos(), centre[1] + r * a.sin()]
        })
        .collect();
    let mut curves = Vec::new();
    for i in 0..n {
        let (a, b) = (corners[i], corners[(i + 1) % n]);
        let id = 10 + 3 * i as u64;
        let choice = rng.below(10);
        if choice < 6 {
            // Either way round: the direction a line is drawn in must not
            // matter.
            curves.push(if rng.below(2) == 0 {
                line(id, a, b)
            } else {
                line(id, b, a)
            });
            continue;
        }
        let chord = [b[0] - a[0], b[1] - a[1]];
        let length = chord[0].hypot(chord[1]);
        let sagitta = length * between(rng, 0.03, 0.2);
        let radius = (length * length / 4.0 + sagitta * sagitta) / (2.0 * sagitta);
        let left = [-chord[1] / length, chord[0] / length];
        let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let outward = choice < 8;
        // The centre is on the inside (the left) for a bulge outward, and
        // the arc then runs counter-clockwise from a to b.
        let reach = if outward {
            radius - sagitta
        } else {
            -(radius - sagitta)
        };
        let c = [middle[0] + left[0] * reach, middle[1] + left[1] * reach];
        curves.push(if outward {
            arc(id, c, a, b)
        } else {
            arc(id, c, b, a)
        });
    }
    (curves, corners)
}

/// A profile drawn against the axis: an arc about a point of the axis
/// between two latitudes, joined to the axis by a line at each end that is
/// not a pole, and the axis between. `offset` moves it off the axis.
fn turned(rng: &mut Rng, axis: Axis, side: f64, size: f64, offset: f64) -> Vec<SketchCurve> {
    let radius = size * between(rng, 0.5, 1.5);
    let h = size * between(rng, -1.0, 1.0);
    let low = if rng.below(3) == 0 {
        -FRAC_PI_2
    } else {
        between(rng, -1.4, 0.2)
    };
    let high = if rng.below(3) == 0 {
        FRAC_PI_2
    } else {
        between(rng, low + 0.3, 1.45)
    };
    if low == -FRAC_PI_2 && high == FRAC_PI_2 && offset == 0.0 {
        return half_disc(axis, side, h, radius);
    }
    let at = |latitude: f64| {
        (
            offset + radius * latitude.cos(),
            h + radius * latitude.sin(),
        )
    };
    let (p1, p2) = (at(low), at(high));
    // Where the profile meets the axis below and above the arc: level with
    // the arc's end (a disc) or further out (a cone).
    let below = p1.1
        - if rng.below(2) == 0 {
            0.0
        } else {
            size * rng.unit()
        };
    let above = p2.1
        + if rng.below(2) == 0 {
            0.0
        } else {
            size * rng.unit()
        };
    let mut points = vec![(offset, below)];
    if low > -FRAC_PI_2 || offset > 0.0 {
        points.push(p1);
    }
    points.push(p2);
    if high < FRAC_PI_2 || offset > 0.0 {
        points.push((offset, above));
    }
    points.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 1e-3 * size);
    let place = |p: (f64, f64)| lathe(axis, side, p.0, p.1);
    let centre = place((offset, h));
    let mut curves = Vec::new();
    let n = points.len();
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        let id = 1 + i as u64;
        if a == p1 && b == p2 {
            // Upwards the arc runs counter-clockwise to the right of the
            // axis, clockwise to its left.
            curves.push(if side < 0.0 {
                arc(id, centre, place(a), place(b))
            } else {
                arc(id, centre, place(b), place(a))
            });
        } else {
            curves.push(line(id, place(a), place(b)));
        }
    }
    curves
}

fn random_frame(rng: &mut Rng, k: f64) -> Frame {
    loop {
        let mut v = || Vec3::new(rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5);
        let (z, x, o) = (v(), v(), v());
        if let Some(frame) = Frame::new(o * (20.0 * k), z, x) {
            return frame;
        }
    }
}

#[derive(Default, Debug)]
struct Tally {
    valid: usize,
    meshed: usize,
    no_profile: usize,
    crosses: usize,
    pinched: usize,
    invalid_angles: usize,
    geometry: usize,
}

#[test]
fn random_profiles_axes_and_sweeps() {
    let mut rng = Rng(0x5EED_0FA7_AB1E);
    let mut tally = Tally::default();
    let mut surfaces: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let runs = 4000;
    for run in 0..runs {
        let k = [1e-3, 1.0, 1e6][rng.below(3) as usize];
        let tol = tolerance(k);
        let plane = random_frame(&mut rng, k);
        let centre = [
            between(&mut rng, -5.0, 5.0) * k,
            between(&mut rng, -5.0, 5.0) * k,
        ];
        let angle = between(&mut rng, 0.0, TAU);
        let direction = [angle.cos() * between(&mut rng, 0.1, 9.0), angle.sin()];
        let direction = if direction[0].hypot(direction[1]) > 0.01 {
            direction
        } else {
            [1.0, 0.0]
        };
        // What to expect: Some(true) must work, Some(false) must cross.
        let (curves, axis, expected): (Vec<SketchCurve>, Axis, Option<bool>) = match rng.below(10) {
            0..=3 => {
                // Clear of the axis, by the box of every whole circle.
                let (curves, _) = blob(&mut rng, centre, 2.0 * k);
                let gap = k * between(&mut rng, 0.05, 4.0);
                let axis = clear_axis(&curves, direction, gap);
                (curves, axis, Some(true))
            }
            4 => {
                // Through the middle, which is outside a polygon whose
                // corners leave more than half a turn between two of them:
                // it must cross the axis when its curves, sampled, are
                // clearly on both sides.
                let (curves, _) = blob(&mut rng, centre, 2.0 * k);
                let axis = Axis {
                    point: centre,
                    direction,
                };
                let crosses = straddles(&curves, axis, 1e-3 * k);
                (curves, axis, crosses.then_some(false))
            }
            5 => {
                // Along a side of the polygon: on the axis when the side
                // is a line.
                let (curves, corners) = blob(&mut rng, centre, 2.0 * k);
                let i = rng.below(corners.len() as u64) as usize;
                let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
                let axis = Axis {
                    point: a,
                    direction: [b[0] - a[0], b[1] - a[1]],
                };
                (curves, axis, None)
            }
            6 => {
                // Through one corner, square to the line from the centre.
                let (curves, corners) = blob(&mut rng, centre, 2.0 * k);
                let a = corners[rng.below(corners.len() as u64) as usize];
                let axis = Axis {
                    point: a,
                    direction: [-(a[1] - centre[1]), a[0] - centre[0]],
                };
                (curves, axis, None)
            }
            7 | 8 => {
                // Drawn against the axis: spheres, cones, discs, poles.
                let axis = Axis {
                    point: centre,
                    direction,
                };
                let side = if rng.below(2) == 0 { 1.0 } else { -1.0 };
                (turned(&mut rng, axis, side, k, 0.0), axis, Some(true))
            }
            _ => {
                // The same, moved off the axis: a torus, or (an arc whose circle reaches the
                // axis) a rational surface.
                let axis = Axis {
                    point: centre,
                    direction,
                };
                let side = if rng.below(2) == 0 { 1.0 } else { -1.0 };
                let offset = k * between(&mut rng, 0.2, 3.0);
                (turned(&mut rng, axis, side, k, offset), axis, None)
            }
        };
        let sweep = if rng.below(3) == 0 {
            TAU
        } else {
            between(&mut rng, 0.05, TAU - 0.01)
        };
        let from = between(&mut rng, -7.0, 7.0);
        let what = format!("run {run}: {curves:?} about {axis:?} from {from} through {sweep}");

        let profiles = match detect_profiles(&curves, &tol) {
            Ok(profiles) => profiles,
            Err(_) => {
                // A bulge that runs into its neighbour, mostly.
                tally.no_profile += 1;
                continue;
            }
        };
        let result = revolve(
            &profiles,
            &plane,
            axis.point,
            axis.direction,
            from,
            from + sweep,
            &tol,
        );
        match result {
            Ok(result) => {
                assert_ne!(expected, Some(false), "{what}: should cross the axis");
                let body = &result.body;
                assert_eq!(check(body, &tol), vec![], "{what}");
                let mass = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
                // Pappus, with the kernel's own exact area and centroid of
                // each profile, and the curves' lengths worked out here.
                let volume: f64 = profiles
                    .iter()
                    .map(|p| p.area() * axis.left(p.centroid().unwrap()).abs() * sweep)
                    .sum();
                let lateral: f64 = curves
                    .iter()
                    .map(|c| {
                        let (length, centroid) = measure(&c.curve);
                        length * axis.left(centroid).abs() * sweep
                    })
                    .sum();
                let caps: f64 = if sweep < TAU {
                    2.0 * profiles.iter().map(|p| p.area()).sum::<f64>()
                } else {
                    0.0
                };
                assert!(
                    (mass.volume - volume).abs() <= 1e-9 * volume,
                    "{what}: volume {} vs {volume}",
                    mass.volume
                );
                assert!(
                    (mass.area - lateral - caps).abs() <= 1e-9 * (lateral + caps),
                    "{what}: area {} vs {}",
                    mass.area,
                    lateral + caps
                );
                tally.valid += 1;
                for face in &body.faces {
                    *surfaces.entry(face.surface.kind()).or_default() += 1;
                }
                if run % 8 == 0 {
                    let chord = 0.01 * k;
                    let params = TessParams {
                        chordal_tolerance: chord,
                        ..TessParams::default()
                    };
                    let mesh =
                        tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let welded = mesh.weld().unwrap();
                    assert!(welded.is_closed(), "{what}: the mesh is not closed");
                    assert!(
                        (welded.volume() - mass.volume).abs() <= mass.area * chord,
                        "{what}: mesh volume {} vs {}",
                        welded.volume(),
                        mass.volume
                    );
                    tally.meshed += 1;
                }
            }
            Err(error) => {
                assert_ne!(expected, Some(true), "{what}: {error}");
                match error {
                    KernelError::CrossesAxis { ref curves } => {
                        assert!(!curves.is_empty(), "{what}");
                        tally.crosses += 1;
                    }
                    KernelError::UnsupportedCurve { .. } => tally.pinched += 1,
                    KernelError::InvalidAngles { .. } => tally.invalid_angles += 1,
                    KernelError::Geometry(_) => tally.geometry += 1,
                    // In particular never `InvalidBody`: that is a bug.
                    other => panic!("{what}: unexpected error {other}"),
                }
            }
        }
    }
    println!("{runs} runs: {tally:?}; faces by surface: {surfaces:?}");
    assert!(tally.valid > runs / 3, "{tally:?}");
    // Arcs on a spindle or horn torus are rational surfaces (since
    // 2026-10-15; before, they were refused).
    assert!(tally.crosses > 0, "{tally:?}");
    for kind in [
        "plane",
        "cylinder",
        "cone",
        "sphere",
        "torus",
        "nurbs surface",
    ] {
        assert!(surfaces.contains_key(kind), "no {kind} in the run");
    }
}
