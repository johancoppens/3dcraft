//! Two freeform surfaces that run side by side a little apart over a large
//! area (2026-10-28, the sea kayak's `Ov3D`: the inner skin of the sheer
//! plate beside the deck's rim, which lies on the sheer plate's outer
//! skin, a plate's thickness away over five metres). Leaving out a cell
//! only where the field's value exceeds the cell's reach needs cells no
//! larger than the gap, more than the marcher allows; where it ran out,
//! a bound on the field's second derivative leaves them out instead.
//!
//! A bicubic B-spline skin against itself lifted by a gap (no meeting),
//! and against itself lifted and tilted so that the two cross along the
//! iso-curve `u = ½` (the lift is linear in x, and x is linear in u): the
//! curve found lies on that iso-curve and is as long. At 1e-3, 1 and 1e6,
//! with the tolerance scaled.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{SurfaceMeeting, bounded_marches, meet_surfaces};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Tolerance, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const L: f64 = 4.0;
const W: f64 = 3.0;
const KNOTS_U: [f64; 11] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0];
const KNOTS_V: [f64; 10] = [0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];

/// The skin at scale `k`, lifted by `lift + tilt (x − L/2)` (scale 1):
/// x and y control coordinates at the Greville abscissae, so x = L u.
fn skin(k: f64, lift: f64, tilt: f64) -> Surface {
    let greville = |knots: &[f64], i: usize| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
    let (nu, nv) = (7, 6);
    let mut ctrl = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let (fi, fj) = (i as f64, j as f64);
            let z = 0.8
                * (0.6 + 0.25 * (1.7 * fi + 0.9 * fj).psin() + 0.1 * (2.3 * fj - 0.4 * fi).pcos());
            let x = L * greville(&KNOTS_U, i);
            ctrl.push(Vec3::new(
                x * k,
                W * greville(&KNOTS_V, j) * k,
                (z + lift + tilt * (x - 0.5 * L)) * k,
            ));
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            3,
            3,
            KNOTS_U.to_vec(),
            KNOTS_V.to_vec(),
            nu,
            nv,
            ctrl,
            vec![1.0; nu * nv],
        )
        .unwrap(),
    )
}

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn around(k: f64) -> Bbox3 {
    Bbox3::from_points([
        Vec3::new(-0.1, -0.1, -0.5) * k,
        Vec3::new(4.1, 3.1, 1.5) * k,
    ])
}

/// A gap of 2000 tolerances (a plate's thickness against a micrometre):
/// nothing.
#[test]
fn surfaces_a_little_apart_over_a_large_area_do_not_meet() {
    for k in SCALES {
        let before = bounded_marches();
        let met = meet_surfaces(
            &skin(k, 0.0, 0.0),
            &skin(k, 2e-3, 0.0),
            &around(k),
            &tolerance(k),
        );
        assert_eq!(met, Ok(SurfaceMeeting::None), "at scale {k}");
        // The first march ran out of cells; the second one answered.
        assert_eq!(bounded_marches(), before + 1, "at scale {k}");
    }
}

/// Lifted and tilted by a thousandth: they cross along `u = ½`.
#[test]
fn surfaces_that_cross_at_a_shallow_angle_meet_along_the_curve() {
    for k in SCALES {
        let tol = tolerance(k);
        let (a, b) = (skin(k, 0.0, 0.0), skin(k, 0.0, 1e-3));
        let before = bounded_marches();
        let met = meet_surfaces(&a, &b, &around(k), &tol).unwrap();
        assert_eq!(bounded_marches(), before + 1, "at scale {k}");
        let SurfaceMeeting::Meets(meeting) = met else {
            panic!("at scale {k}: {met:?}");
        };
        let mut length = 0.0;
        for c in &meeting.curves {
            let n = 200;
            let mut last: Option<Vec3> = None;
            for i in 0..=n {
                let t = c.range.0 + (c.range.1 - c.range.0) * f64::from(i) / f64::from(n);
                let p = c.curve.eval(t);
                // On x = L/2, and on both surfaces.
                assert!(
                    (p.x - 0.5 * L * k).abs() <= 2e3 * tol.linear,
                    "at scale {k}: {p:?} off the crossing"
                );
                for s in [&a, &b] {
                    let (u, v) = s.project(p).unwrap();
                    assert!(
                        s.eval(u, v).distance(p) <= tol.linear,
                        "at scale {k}: {p:?} off a surface"
                    );
                }
                if let Some(q) = last {
                    length += p.distance(q);
                }
                last = Some(p);
            }
        }
        // The iso-curve's own length, by many chords.
        let mut iso = 0.0;
        let n = 4000;
        for i in 0..n {
            let (s, t) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            iso += a.eval(0.5, s).distance(a.eval(0.5, t));
        }
        assert!(
            (length - iso).abs() <= 1e-4 * iso,
            "at scale {k}: {length} against {iso}"
        );
    }
}

/// Uniform clamped cubic knots for `count` control points over [0, 1].
fn uniform(count: usize) -> Vec<f64> {
    let spans = count - 3;
    let mut knots = vec![0.0; 4];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend([1.0; 4]);
    knots
}

/// A finer skin (15 × 13 control points, x and y at the Greville
/// abscissae), lifted by `lift`, with the control point (7, 6) moved down
/// by `dip` (scale 1).
fn fine(k: f64, lift: f64, dip: f64) -> Surface {
    let (nu, nv) = (15, 13);
    let (ku, kv) = (uniform(nu), uniform(nv));
    let greville = |knots: &[f64], i: usize| (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
    let mut ctrl = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let (x, y) = (L * greville(&ku, i), W * greville(&kv, j));
            let mut z = 0.5 + 0.2 * (1.1 * x + 0.3).psin() * (0.8 * y - 0.2).pcos() + lift;
            if (i, j) == (7, 6) {
                z -= dip;
            }
            ctrl.push(Vec3::new(x * k, y * k, z * k));
        }
    }
    Surface::Nurbs(NurbsSurface::new(3, 3, ku, kv, nu, nv, ctrl, vec![1.0; nu * nv]).unwrap())
}

/// Side by side 2000 tolerances apart, except where one control point of
/// the upper surface is pulled down by three times the gap: there it dips
/// through the lower one in a small closed loop (where the dip's basis
/// function, at most 4/9, exceeds a third), which the second march, that
/// leaves out the cells of the surfaces side by side, must keep. Every
/// point of it is where the two surfaces' heights at the same parameters
/// agree (they differ only in z, by the gap less the dip).
#[test]
fn a_small_loop_where_surfaces_side_by_side_dip_through_is_kept() {
    for k in SCALES {
        let tol = tolerance(k);
        let gap = 2e-3;
        let (a, b) = (fine(k, 0.0, 0.0), fine(k, gap, 3.0 * gap));
        let before = bounded_marches();
        let met = meet_surfaces(&a, &b, &around(k), &tol).unwrap();
        assert_eq!(bounded_marches(), before + 1, "at scale {k}");
        let SurfaceMeeting::Meets(meeting) = met else {
            panic!("at scale {k}: {met:?}");
        };
        assert_eq!(meeting.curves.len(), 1, "at scale {k}: {meeting:?}");
        let c = &meeting.curves[0];
        assert!(c.closed, "at scale {k}: the loop is not closed");
        let mut length = 0.0;
        let mut last: Option<Vec3> = None;
        for i in 0..=400 {
            let t = c.range.0 + (c.range.1 - c.range.0) * f64::from(i) / 400.0;
            let p = c.curve.eval(t);
            let (u, v) = (p.x / (L * k), p.y / (W * k));
            let gap_here = b.eval(u, v).z - a.eval(u, v).z;
            // Within the tolerance of both surfaces, which cross at a
            // slope of about a hundredth here: the heights agree to a few
            // hundred tolerances.
            assert!(
                gap_here.abs() <= 500.0 * tol.linear,
                "at scale {k}: {p:?} where the surfaces are {gap_here} apart"
            );
            if let Some(q) = last {
                length += p.distance(q);
            }
            last = Some(p);
        }
        // A small loop: longer than the tolerance by far, within the dip's
        // support (four spans each way).
        assert!(
            length > 1e4 * tol.linear && length < 4.0 * k,
            "at scale {k}: the loop is {length} long"
        );
    }
}
