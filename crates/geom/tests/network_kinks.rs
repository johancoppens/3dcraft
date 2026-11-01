//! A sheet held in curvature (G2) along a parameter line of a cubic spline
//! face with knots spaced unevenly (the fuel tank's side against its
//! crown, 2026-11-01): the second derivative across that the face asks
//! for has a kink at every simple knot (`b² C″` of a cubic line), and
//! spread over the sheet by the fair profile it creased the sheet inside,
//! along every such knot (2.6e-3 across the middle of the tank's side),
//! so its offset tore by that angle times the distance and could not be
//! fitted. The kinks now go on a band of their own beside the seam
//! (docs/design/surfacing.md › *Kinks of a curvature hold*). At three
//! scales:
//! - the seam is still proven G2;
//! - the sheet's normal is continuous across every knot inside it to
//!   within ten times the angular tolerance (1.5e-2 without the band);
//! - its offset into the side it bends to, a tenth of its least radius of
//!   curvature, is fitted within a quarter of the tolerance.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::network::{Condition, Network, Order, gordon, gordon_across, iso_curve};
use craft3d_geom::{NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::Vec3;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const ANGULAR: f64 = 1e-6;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn bezier(points: [Vec3; 4]) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points.to_vec(),
    )
    .unwrap()
}

/// A crown: a cubic spline over x in [−r, r] (along u, ten control points
/// on the tank crown's uneven knots, every other one lifted, so its third
/// derivative jumps at each) and y in [−0.6 r, 0.6 r] (a Bézier across),
/// on the sphere of radius `2r` below.
fn crown(r: f64) -> NurbsSurface {
    let (nu, nv) = (10, 4);
    let mut points = Vec::with_capacity(nu * nv);
    for i in 0..nu {
        for j in 0..nv {
            let x = -r + 2.0 * r * i as f64 / (nu - 1) as f64;
            let y = -0.6 * r + 1.2 * r * j as f64 / (nv - 1) as f64;
            let big = 2.0 * r;
            let lift = if i % 2 == 0 { 0.04 * r } else { -0.04 * r };
            let z = (big * big - x * x - y * y).sqrt() - big + lift;
            points.push(v(x, y, z));
        }
    }
    let knots_u = vec![
        0.0, 0.0, 0.0, 0.0, 0.2, 0.222, 0.4, 0.6, 0.778, 0.8, 1.0, 1.0, 1.0, 1.0,
    ];
    let knots_v = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    NurbsSurface::new(3, 3, knots_u, knots_v, nu, nv, points, vec![1.0; nu * nv]).unwrap()
}

/// The side: from the crown's border `y = 0.6 r` (its line v = 1) out and
/// down to a floor line, crossed by four cubics; the floor edge free.
fn side(r: f64, crown: &NurbsSurface) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    side_from(r, iso_curve(crown, true, 1.0).unwrap())
}

/// [`side`] from the crown's edge `edge` (along x at y = 0.6 r).
fn side_from(r: f64, edge: NurbsCurve) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let floor = NurbsCurve::new(
        edge.degree(),
        edge.knots().to_vec(),
        edge.control_points()
            .iter()
            .map(|p| v(p.x, 0.9 * r, -0.8 * r))
            .collect(),
        edge.weights().to_vec(),
    )
    .unwrap();
    let (a, b) = edge.domain();
    let vs: Vec<NurbsCurve> = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]
        .iter()
        .map(|f| {
            let t = a + (b - a) * f;
            let (p, q) = (edge.eval(t), floor.eval(t));
            bezier([
                p,
                p + v(0.0, 0.15 * r, -0.1 * r),
                q + v(0.0, -0.05 * r, 0.3 * r),
                q,
            ])
        })
        .collect();
    (vec![edge, floor], vs)
}

/// The largest angle between the sheet's normals on either side of each
/// of its knots along u that is held only in position (multiplicity at
/// least its degree less one), at 41 places across.
fn creases(s: &NurbsSurface) -> f64 {
    let (pu, _) = s.degrees();
    let knots = s.knots_u();
    let ((u0, u1), (v0, v1)) = s.domain();
    let mut worst = 0.0_f64;
    let mut i = 0;
    while i < knots.len() {
        let k = knots[i];
        let m = knots[i..].iter().take_while(|x| **x == k).count();
        i += m;
        if !(k > u0 && k < u1) || m + 1 < pu {
            continue;
        }
        let e = 1e-9 * (u1 - u0);
        for j in 0..=40 {
            let t = v0 + (v1 - v0) * f64::from(j) / 40.0;
            let normal = |u: f64| {
                let (_, du, dv) = s.derivatives(u, t);
                du.cross(dv).normalized().unwrap()
            };
            worst = worst.max(normal(k - e).cross(normal(k + e)).length());
        }
    }
    worst
}

#[test]
fn a_sheet_held_in_curvature_to_a_cubic_crown_is_smooth_inside_and_offsets() {
    for scale in SCALES {
        let r = scale;
        let tol = 1e-6 * scale;
        let face = crown(r);
        let wrapped = Surface::Nurbs(face.clone());
        let (us, vs) = side(r, &face);
        let patch = gordon(&Network {
            u: &us,
            v: &vs,
            edges: [
                Condition::Curvature {
                    surface: &wrapped,
                    weight: 1.0,
                },
                Condition::Free,
                Condition::Free,
                Condition::Free,
            ],
            tolerance: tol,
            angular: ANGULAR,
        })
        .unwrap();
        let seam = patch.seams[0].expect("the seam is measured");
        assert_eq!(seam.order, Order::G2, "scale {scale}: {seam:?}");
        // Smooth inside: the normal continuous across every knot to within
        // ten times the angular tolerance (the band is kept wide enough for
        // the seam's rounding, so the crease is a little more than the
        // share asked; without the band it is 1.5e-2 here).
        let crease = creases(&patch.surface);
        assert!(
            crease <= 10.0 * ANGULAR,
            "scale {scale}: creased by {crease:e}"
        );
        // Its offset at a tenth of the side's size toward the side it
        // bends to is fitted within a quarter of the tolerance.
        let sheet = Surface::Nurbs(patch.surface.clone());
        for distance in [0.02 * r, -0.02 * r] {
            let fitted = craft3d_geom::offset::offset(&sheet, distance, 0.25 * tol);
            assert!(
                fitted.is_ok(),
                "scale {scale}, distance {distance:e}: {fitted:?}"
            );
            let fitted = fitted.unwrap();
            for (a, b) in [(0.1, 0.2), (0.5, 0.5), (0.79, 0.9), (0.81, 0.05)] {
                let exact = craft3d_geom::offset::exact(&sheet, distance, a, b).unwrap();
                let off = fitted.eval(a, b).distance(exact);
                assert!(off <= 0.25 * tol * 1.01, "scale {scale}: off by {off:e}");
            }
        }
    }
}

/// The crown of a network: six sections across x and three lines along it,
/// each of `degree` through points of `z = √(4r² − x² − y²) − 2r` with a
/// ripple along x, crossing exactly; interpolated across with `across`.
fn network_crown(r: f64, degree: usize, across: usize) -> NurbsSurface {
    let f = |x: f64, y: f64| {
        let big = 2.0 * r;
        (big * big - x * x - y * y).sqrt() - big + 0.1 * r * (6.0 * x / r).psin() * (1.0 + y / r)
    };
    let curve = |pts: Vec<Vec3>| -> NurbsCurve {
        let n = pts.len() - 1;
        let params: Vec<f64> = (0..=n).map(|k| k as f64 / n as f64).collect();
        let values: Vec<Vec<f64>> = pts.iter().map(|p| vec![p.x, p.y, p.z]).collect();
        let (knots, ctrl) = craft3d_geom::refine::interpolate(&params, degree, &values).unwrap();
        NurbsCurve::polynomial(
            degree,
            knots,
            ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect(),
        )
        .unwrap()
    };
    // Sections at x = −r, −0.6 r, …, r (along y, seven points each, the
    // three lines' y among them); lines at y = −0.6 r, 0, 0.6 r (along x,
    // eleven points each, the sections' x among them).
    let us: Vec<NurbsCurve> = (0..6)
        .map(|i| {
            let x = -r + 0.4 * r * f64::from(i);
            curve(
                (0..7)
                    .map(|j| {
                        let y = -0.6 * r + 0.2 * r * f64::from(j);
                        v(x, y, f(x, y))
                    })
                    .collect(),
            )
        })
        .collect();
    let vs: Vec<NurbsCurve> = [-0.6 * r, 0.0, 0.6 * r]
        .iter()
        .map(|&y| {
            curve(
                (0..11)
                    .map(|k| {
                        let x = -r + 0.2 * r * f64::from(k);
                        v(x, y, f(x, y))
                    })
                    .collect(),
            )
        })
        .collect();
    gordon_across(
        &Network {
            u: &us,
            v: &vs,
            edges: [Condition::Free; 4],
            tolerance: 1e-6 * r,
            angular: ANGULAR,
        },
        across,
    )
    .unwrap()
    .surface
}

/// The crown's parameter line along x at y = 0.6 r.
fn far_edge(r: f64, crown: &NurbsSurface) -> NurbsCurve {
    for (along_u, at) in [(true, 1.0), (true, 0.0), (false, 1.0), (false, 0.0)] {
        if let Ok(c) = iso_curve(crown, along_u, at) {
            let (a, b) = c.domain();
            let (p, q) = (c.eval(a), c.eval(b));
            if (p.y - 0.6 * r).abs() < 1e-9 * r && (q.y - 0.6 * r).abs() < 1e-9 * r {
                let _ = q;
                return c;
            }
        }
    }
    panic!("the crown has no line at y = 0.6 r")
}

/// Whether the sheet has knots across close to its held edge (`v` near 0):
/// the band a curvature hold puts the kinks of its change on.
fn banded(s: &NurbsSurface) -> bool {
    let (_, (v0, v1)) = s.domain();
    s.knots_v()
        .iter()
        .any(|k| *k > v0 && *k < v0 + 0.1 * (v1 - v0))
}

/// A crown interpolated across with degree 5 from curves of degree 5 is
/// four times differentiable along its lines: a side held in curvature to
/// it needs no band for kinks and has no crease inside (`acrossDegree` 5,
/// format 62; the fuel tank with quintic shoulders and spine measured so by
/// hand). Made of cubics interpolated across with degree 3, the crown's
/// line kinks and the side's change is put onto a band. The seam is held at a tolerance of 1e-6 of the size
/// and an angular tolerance of 1e-6 (at the fuel tank's default 1e-9 rad
/// its quintic seam is not proven G2 yet: docs/design/surfacing.md).
#[test]
fn a_side_held_in_curvature_to_a_quintic_crown_needs_no_band_and_is_smooth_inside() {
    for scale in SCALES {
        let r = scale;
        let tol = 1e-6 * scale;
        for across in [5, 3] {
            let crown = network_crown(r, across, across);
            let wrapped = Surface::Nurbs(crown.clone());
            let (us, vs) = side_from(r, far_edge(r, &crown));
            let patch = gordon(&Network {
                u: &us,
                v: &vs,
                edges: [
                    Condition::Curvature {
                        surface: &wrapped,
                        weight: 1.0,
                    },
                    Condition::Free,
                    Condition::Free,
                    Condition::Free,
                ],
                tolerance: tol,
                angular: ANGULAR,
            })
            .unwrap();
            let seam = patch.seams[0].expect("the seam is measured");
            assert_eq!(
                seam.order,
                Order::G2,
                "scale {scale}, across {across}: {seam:?}"
            );
            if across == 5 {
                assert!(
                    !banded(&patch.surface),
                    "scale {scale}: a band on a quintic crown"
                );
                let crease = creases(&patch.surface);
                assert!(crease <= ANGULAR, "scale {scale}: creased by {crease:e}");
            } else {
                assert!(
                    banded(&patch.surface),
                    "scale {scale}: no band on a cubic crown"
                );
            }
        }
    }
}
