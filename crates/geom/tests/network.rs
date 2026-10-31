//! Surfaces through curve networks (`network`): closed forms at three
//! scales. A Coons patch reproduces the bilinear patch of its sides; a
//! Gordon surface reproduces a tensor B-spline from its own iso-curves and
//! the exact rational cylinder and sphere from their circles and lines;
//! a patch held tangent meets its neighbour (another network patch, a
//! cylinder) with one tangent plane all along the seam; inconsistent
//! networks are refused with the curves named. Changing a curve's
//! description (degree elevation, knot insertion, one knot vector for
//! several) leaves every point where it was.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::network::{
    Condition, Network, NetworkError, arc, chain, coons, gordon, iso_curve, seam_gaps,
};
use craft3d_geom::refine::{compatible, elevated, interpolate, with_knots};
use craft3d_geom::{Curve3, Cylinder, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const FREE: [Condition<'static>; 4] = [Condition::Free; 4];

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

fn spline(points: &[Vec3]) -> NurbsCurve {
    let n = points.len();
    let p = (n - 1).min(3);
    let mut knots = vec![0.0; p + 1];
    for k in 1..n - p {
        knots.push(k as f64 / (n - p) as f64);
    }
    knots.extend(vec![1.0; p + 1]);
    NurbsCurve::polynomial(p, knots, points.to_vec()).unwrap()
}

/// The cubic through the points at the parameters.
fn through(params: &[f64], points: &[Vec3]) -> NurbsCurve {
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(params, 3, &values).unwrap();
    NurbsCurve::polynomial(3, knots, ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect()).unwrap()
}

fn net<'a>(
    u: &'a [NurbsCurve],
    vv: &'a [NurbsCurve],
    edges: [Condition<'a>; 4],
    scale: f64,
) -> Network<'a> {
    Network {
        u,
        v: vv,
        edges,
        tolerance: 1e-6 * scale,
        angular: 1e-9,
    }
}

fn grid() -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for i in 0..=8 {
        for j in 0..=8 {
            out.push((f64::from(i) / 8.0, f64::from(j) / 8.0));
        }
    }
    out
}

fn normal(s: &NurbsSurface, u: f64, w: f64) -> Option<Vec3> {
    let (_, a, b) = s.derivatives(u, w);
    a.cross(b).normalized()
}

#[test]
fn elevation_and_knots_keep_every_point() {
    for scale in SCALES {
        let frame = Frame::WORLD;
        let quarter = arc(&frame, 2.0 * scale, 0.3, 2.0).unwrap();
        let wave = spline(&[
            v(0.0, 0.0, 0.0) * scale,
            v(1.0, 2.0, 0.5) * scale,
            v(2.0, -1.0, 1.0) * scale,
            v(3.0, 0.5, 0.0) * scale,
            v(4.0, 0.0, 0.2) * scale,
        ]);
        for curve in [&quarter, &wave] {
            let (a, b) = curve.domain();
            let up = elevated(curve, curve.degree() + 2).unwrap();
            let more = with_knots(
                curve,
                &[a + 0.3 * (b - a), a + 0.3 * (b - a), a + 0.71 * (b - a)],
            )
            .unwrap();
            for k in 0..=64 {
                let t = a + (b - a) * f64::from(k) / 64.0;
                let p = curve.eval(t);
                let (c, d) = up.domain();
                assert!(up.eval(c + (d - c) * f64::from(k) / 64.0).distance(p) <= 1e-12 * scale);
                assert!(more.eval(t).distance(p) <= 1e-12 * scale);
            }
        }
        let both = compatible(&[quarter.clone(), wave.clone()]).unwrap();
        assert_eq!(both[0].degree(), both[1].degree());
        assert_eq!(both[0].knots(), both[1].knots());
        for (made, curve) in both.iter().zip([&quarter, &wave]) {
            let (a, b) = curve.domain();
            for k in 0..=64 {
                let t = f64::from(k) / 64.0;
                assert!(made.eval(t).distance(curve.eval(a + (b - a) * t)) <= 1e-12 * scale);
            }
        }
    }
}

#[test]
fn a_coons_patch_reproduces_the_bilinear_patch_of_its_sides() {
    for scale in SCALES {
        let p00 = v(0.0, 0.0, 0.0) * scale;
        let p10 = v(3.0, 0.2, 1.0) * scale;
        let p01 = v(-0.5, 2.0, 0.4) * scale;
        let p11 = v(2.5, 2.5, -1.0) * scale;
        // One side given the other way round, one as a cubic.
        let bottom = elevated(&line(p00, p10), 3).unwrap();
        let top = line(p11, p01);
        let left = line(p00, p01);
        let right = line(p10, p11);
        let patch = coons([&bottom, &top], [&left, &right], FREE, 1e-6 * scale, 1e-9).unwrap();
        assert_eq!(patch.reversed_u, vec![false, true]);
        for (s, t) in grid() {
            let want = (p00 * (1.0 - s) + p10 * s) * (1.0 - t) + (p01 * (1.0 - s) + p11 * s) * t;
            let got = patch.surface.eval(s, t);
            assert!(
                got.distance(want) <= 1e-12 * scale,
                "at ({s}, {t}): {got:?} against {want:?}"
            );
        }
        assert!(patch.mismatch <= 1e-12 * scale);
    }
}

fn tensor(scale: f64) -> NurbsSurface {
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
    let mut ctrl = Vec::new();
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (f64::from(i), f64::from(j));
            ctrl.push(
                v(
                    x,
                    y,
                    (0.7 * x).psin() * (0.9 * y + 0.3).pcos() + 0.1 * x * y,
                ) * scale,
            );
        }
    }
    NurbsSurface::new(3, 3, knots.clone(), knots, 5, 5, ctrl, vec![1.0; 25]).unwrap()
}

#[test]
fn a_gordon_surface_reproduces_a_tensor_spline_from_its_iso_curves() {
    for scale in SCALES {
        let s = tensor(scale);
        let at = [0.0, 0.25, 0.5, 0.75, 1.0];
        let us: Vec<NurbsCurve> = at
            .iter()
            .map(|c| iso_curve(&s, true, *c).unwrap())
            .collect();
        let vs: Vec<NurbsCurve> = at
            .iter()
            .map(|c| iso_curve(&s, false, *c).unwrap())
            .collect();
        let patch = gordon(&net(&us, &vs, FREE, scale)).unwrap();
        for (a, b) in grid() {
            let d = patch.surface.eval(a, b).distance(s.eval(a, b));
            assert!(d <= 1e-11 * scale, "at ({a}, {b}): {d:e}");
        }
    }
}

/// Quarter circles of radius `r` about the z axis at the heights, and the
/// lines along the cylinder at the angles (as fractions of the quarter).
fn cylinder_network(r: f64, heights: &[f64], angles: &[f64]) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let us = heights
        .iter()
        .map(|h| {
            let frame = Frame::new(v(0.0, 0.0, *h), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
            arc(&frame, r, 0.0, FRAC_PI_2).unwrap()
        })
        .collect();
    let (h0, h1) = (heights[0], *heights.last().unwrap());
    let vs = angles
        .iter()
        .map(|f| {
            // The point of the rational quarter at parameter f.
            let q = arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap().eval(*f);
            line(v(q.x, q.y, h0), v(q.x, q.y, h1))
        })
        .collect();
    (us, vs)
}

#[test]
fn a_network_of_circles_and_lines_is_the_cylinder() {
    for scale in SCALES {
        let r = 2.0 * scale;
        for (heights, angles) in [
            (vec![0.0, 3.0 * scale], vec![0.0, 1.0]),
            (vec![0.0, 1.0 * scale, 3.0 * scale], vec![0.0, 0.5, 1.0]),
        ] {
            let (us, vs) = cylinder_network(r, &heights, &angles);
            let patch = gordon(&net(&us, &vs, FREE, scale)).unwrap();
            for (a, b) in grid() {
                let p = patch.surface.eval(a, b);
                let radial = v(p.x, p.y, 0.0);
                assert!(
                    (radial.length() - r).abs() <= 1e-12 * scale,
                    "off the cylinder by {:e}",
                    radial.length() - r
                );
                let n = normal(&patch.surface, a, b).unwrap();
                let sine = n.cross(radial.normalized().unwrap()).length();
                assert!(sine <= 1e-9, "tangent plane off by {sine:e}");
            }
        }
    }
}

#[test]
fn a_network_of_meridians_and_parallels_is_the_sphere_octant() {
    for scale in SCALES {
        let r = 1.5 * scale;
        // Meridians from the equator to the pole at 0, 45 and 90 degrees.
        let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
            .iter()
            .map(|f| {
                let q = arc(&Frame::WORLD, 1.0, 0.0, FRAC_PI_2).unwrap().eval(*f);
                let frame =
                    Frame::new(v(0.0, 0.0, 0.0), v(q.y, -q.x, 0.0), v(q.x, q.y, 0.0)).unwrap();
                arc(&frame, r, 0.0, FRAC_PI_2).unwrap()
            })
            .collect();
        let middle = vs[0].eval(0.5);
        let parallel =
            Frame::new(v(0.0, 0.0, middle.z), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
        let pole = v(0.0, 0.0, r);
        let us = vec![
            arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
            arc(&parallel, middle.x, 0.0, FRAC_PI_2).unwrap(),
            line(pole, pole),
        ];
        let patch = gordon(&net(&us, &vs, FREE, scale)).unwrap();
        assert!(patch.boundary[1].is_none(), "the pole is a point");
        for (a, b) in grid() {
            let p = patch.surface.eval(a, b);
            assert!(
                (p.length() - r).abs() <= 1e-12 * scale,
                "off the sphere by {:e}",
                p.length() - r
            );
            if b < 1.0 {
                let n = normal(&patch.surface, a, b).unwrap();
                let sine = n.cross(p.normalized().unwrap()).length();
                assert!(sine <= 1e-9, "tangent plane off by {sine:e} at ({a}, {b})");
            }
        }
    }
}

fn wavy(scale: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let h = |x: f64, y: f64| 0.3 * (1.3 * x).psin() + 0.2 * (0.8 * y).pcos() + 0.05 * x * y;
    let at = |x: f64, y: f64| v(x, y, h(x, y)) * scale;
    let xs = [0.0, 1.0, 2.0];
    let ys = [0.0, 1.5, 3.0];
    let us = ys
        .iter()
        .map(|y| {
            spline(
                &(0..=4)
                    .map(|k| at(0.5 * f64::from(k), *y))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    // The v curves through the u curves' own points where they cross.
    let vs = xs
        .iter()
        .map(|x| {
            let pts: Vec<Vec3> = us.iter().map(|c: &NurbsCurve| c.eval(*x / 2.0)).collect();
            through(
                &[0.0, 0.25, 0.5, 1.0],
                &[
                    pts[0],
                    (pts[0] + pts[1]) * 0.5 + v(0.0, 0.0, 0.2) * scale,
                    pts[1],
                    pts[2],
                ],
            )
        })
        .collect::<Vec<_>>();
    (us, vs)
}

#[test]
fn two_network_patches_meet_tangentially_where_one_is_held_to_the_other() {
    for scale in SCALES {
        let (us, vs) = wavy(scale);
        let first = gordon(&net(&us, &vs, FREE, scale)).unwrap();
        // The second patch starts at the first one's last u curve and
        // goes on in y; its v curves leave the seam at an angle.
        let seam = first.boundary[1].clone().unwrap();
        let far = |x: f64| v(x, 5.0, 0.8 + 0.1 * x) * scale;
        let (a, b) = seam.domain();
        let ends = [seam.eval(a), seam.eval(0.5 * (a + b)), seam.eval(b)];
        let next_vs: Vec<NurbsCurve> = ends
            .iter()
            .zip([0.0, 1.0, 2.0])
            .map(|(p, x)| spline(&[*p, *p + v(0.0, 0.7, 0.6) * scale, far(x)]))
            .collect();
        let next_us = vec![seam.clone(), spline(&[far(0.0), far(1.0), far(2.0)])];
        let face = Surface::Nurbs(first.surface.clone());
        let edges = [
            Condition::Tangent {
                surface: &face,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Free,
            Condition::Free,
        ];
        let second = gordon(&net(&next_us, &next_vs, edges, scale)).unwrap();
        assert!(second.tangency[0].unwrap() <= 1e-9);
        let own = Surface::Nurbs(second.surface.clone());
        let gaps = seam_gaps(
            &Curve3::Nurbs(seam.clone()),
            seam.domain(),
            &face,
            &own,
            513,
        )
        .unwrap();
        assert!(
            gaps.position <= 1e-9 * scale,
            "apart by {:e}",
            gaps.position
        );
        assert!(
            gaps.angle <= 1e-9,
            "creased by {:e} at {:?}",
            gaps.angle,
            gaps.angle_at
        );
        // Without the condition the seam is creased: the test can see one.
        let creased = gordon(&net(&next_us, &next_vs, FREE, scale)).unwrap();
        let plain = Surface::Nurbs(creased.surface.clone());
        let gaps = seam_gaps(
            &Curve3::Nurbs(seam.clone()),
            seam.domain(),
            &face,
            &plain,
            513,
        )
        .unwrap();
        assert!(gaps.angle > 1e-2, "{:e}", gaps.angle);
        // The far u curve is still the surface's edge (the v curves were
        // bent to leave the seam along the first patch).
        for k in 0..=16 {
            let t = f64::from(k) / 16.0;
            let p = next_us[1].eval(t);
            assert!(second.surface.eval(t, 1.0).distance(p) <= 1e-9 * scale);
        }
    }
}

#[test]
fn a_patch_held_tangent_to_a_cylinder_meets_it_with_one_tangent_plane() {
    for scale in SCALES {
        let r = 2.0 * scale;
        let h = 3.0 * scale;
        let cylinder = Surface::Cylinder(Cylinder::new(Frame::WORLD, r).unwrap());
        // Along the cylinder's line at angle 0, out towards +y and +x.
        let seam = line(v(r, 0.0, 0.0), v(r, 0.0, h));
        let far = |z: f64| v(r + 1.5 * scale, 2.5 * scale, z + 0.2 * scale);
        let vs: Vec<NurbsCurve> = [0.0, 0.5 * h, h]
            .iter()
            .map(|z| spline(&[v(r, 0.0, *z), v(r + 0.6 * scale, 1.2 * scale, *z), far(*z)]))
            .collect();
        let us = vec![seam.clone(), line(far(0.0), far(h))];
        let edges = [
            Condition::Tangent {
                surface: &cylinder,
                weight: 1.0,
            },
            Condition::Free,
            Condition::Free,
            Condition::Free,
        ];
        let patch = gordon(&net(&us, &vs, edges, scale)).unwrap();
        assert!(patch.tangency[0].unwrap() <= 1e-9);
        let own = Surface::Nurbs(patch.surface.clone());
        let gaps = seam_gaps(
            &Curve3::Nurbs(seam.clone()),
            seam.domain(),
            &cylinder,
            &own,
            513,
        )
        .unwrap();
        assert!(gaps.position <= 1e-9 * scale);
        assert!(gaps.angle <= 1e-9, "creased by {:e}", gaps.angle);
        // The tangent-magnitude weight changes the surface, not the seam.
        let edges = [
            Condition::Tangent {
                surface: &cylinder,
                weight: 2.0,
            },
            Condition::Free,
            Condition::Free,
            Condition::Free,
        ];
        let fuller = gordon(&net(&us, &vs, edges, scale)).unwrap();
        let own2 = Surface::Nurbs(fuller.surface.clone());
        let gaps = seam_gaps(
            &Curve3::Nurbs(seam.clone()),
            seam.domain(),
            &cylinder,
            &own2,
            513,
        )
        .unwrap();
        assert!(gaps.angle <= 1e-9);
        assert!(
            fuller
                .surface
                .eval(0.5, 0.1)
                .distance(patch.surface.eval(0.5, 0.1))
                > 1e-3 * scale
        );
    }
}

fn refused(result: Result<impl std::fmt::Debug, NetworkError>) -> String {
    match result {
        Err(NetworkError::Refused(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn curves_that_do_not_cross_are_refused_by_name() {
    for scale in SCALES {
        let (us, mut vs) = wavy(scale);
        let off = vs[1]
            .control_points()
            .iter()
            .map(|p| *p + v(0.0, 0.0, 1e-4 * scale))
            .collect();
        vs[1] = NurbsCurve::polynomial(vs[1].degree(), vs[1].knots().to_vec(), off).unwrap();
        let why = refused(gordon(&net(&us, &vs, FREE, scale)));
        assert!(
            why.contains("do not cross") && why.contains("v curve 2"),
            "{why}"
        );
    }
}

#[test]
fn an_inconsistent_network_is_refused_with_its_reason() {
    for scale in SCALES {
        // Out of order: the u curves listed against the v curves' order.
        let (mut us, vs) = wavy(scale);
        us.swap(0, 1);
        let why = refused(gordon(&net(&us, &vs, FREE, scale)));
        assert!(
            why.contains("out of order") || why.contains("runs past"),
            "{why}"
        );
        // Weights that cannot agree: an arc and a polynomial curve through
        // the same three points, crossed in the middle.
        let r = scale;
        let quarter = arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap();
        let mid = quarter.eval(0.5);
        let lift = v(0.0, 0.0, r);
        let other = NurbsCurve::polynomial(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                v(r, 0.0, 0.0) + lift,
                (mid + lift) * 2.0 - (v(r, 0.0, 0.0) + v(0.0, r, 0.0) + lift * 2.0) * 0.5,
                v(0.0, r, 0.0) + lift,
            ],
        )
        .unwrap();
        let us = vec![quarter.clone(), other.clone()];
        let vs = vec![
            line(v(r, 0.0, 0.0), v(r, 0.0, 0.0) + lift),
            line(mid, other.eval(0.5)),
            line(v(0.0, r, 0.0), v(0.0, r, 0.0) + lift),
        ];
        let why = refused(gordon(&net(&us, &vs, FREE, scale)));
        assert!(why.contains("weights"), "{why}");
    }
}

#[test]
fn bad_networks_are_errors_not_panics() {
    let (us, vs) = wavy(1.0);
    assert!(gordon(&net(&us[..1], &vs, FREE, 1.0)).is_err());
    assert!(gordon(&net(&us, &[], FREE, 1.0)).is_err());
    let mut bad = net(&us, &vs, FREE, 1.0);
    bad.tolerance = f64::NAN;
    assert!(gordon(&bad).is_err());
    let face = Surface::Nurbs(tensor(1.0));
    let mut weighted = net(&us, &vs, FREE, 1.0);
    weighted.edges[0] = Condition::Tangent {
        surface: &face,
        weight: -1.0,
    };
    assert!(gordon(&weighted).is_err());
    // A tangent edge along a spline face's curve that is not one of its
    // parameter lines.
    weighted.edges[0] = Condition::Tangent {
        surface: &face,
        weight: 1.0,
    };
    assert!(gordon(&weighted).is_err());
    assert!(chain(&[], 1e-6, 1e-9).is_err());
    assert!(arc(&Frame::WORLD, -1.0, 0.0, 1.0).is_err());
}

#[test]
fn a_chain_of_an_arc_and_its_tangent_line_runs_on_smoothly() {
    for scale in SCALES {
        let r = scale;
        let quarter = arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap();
        let tail = line(v(0.0, r, 0.0), v(-2.0 * r, r, 0.0));
        let joined = chain(&[quarter.clone(), tail], 1e-6 * scale, 1e-9).unwrap();
        // The joint: find it, and compare the derivative on both sides.
        let mut joint = 0.0;
        for k in 0..=1000 {
            let t = f64::from(k) / 1000.0;
            if joined.eval(t).distance(v(0.0, r, 0.0)) < joined.eval(joint).distance(v(0.0, r, 0.0))
            {
                joint = t;
            }
        }
        let (_, before) = joined.derivatives(joint - 1e-9);
        let (_, after) = joined.derivatives(joint + 1e-9);
        assert!(
            (before - after).length() <= 1e-5 * before.length(),
            "{before:?} {after:?}"
        );
        for k in 0..=32 {
            let t = f64::from(k) / 32.0;
            let p = joined.eval(t);
            let on_arc = (p.length() - r).abs() <= 1e-12 * scale && p.y >= -1e-12 * scale;
            let on_line = (p.y - r).abs() <= 1e-12 * scale;
            assert!(on_arc || on_line, "{p:?}");
        }
    }
}
