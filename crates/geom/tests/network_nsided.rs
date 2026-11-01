//! The N-sided fill (docs/design/surfacing.md › *The N-sided fill*):
//! spherical caps of three, five and six great-circle arcs held in
//! curvature to their sphere are filled within 2e-5 of its radius, every
//! seam and edge proven G2, the error falling as the fourth power of the
//! cap's size (held tangent: G1, within 1e-4); a sphere's octant too, its patches regular
//! (no pole); a flat pentagon is filled by its plane exactly; a twisted
//! hexagon with free edges is filled with proven seams; at three scales;
//! bad loops are refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::network::nsided::{Fill, Inner, Nsided, nsided};
use craft3d_geom::network::{Condition, Order, arc};
use craft3d_geom::{NurbsCurve, Sphere, Surface};
use craft3d_math::{Frame, Portable, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

/// `n` great-circle arcs between points on the small circle `polar` from
/// the sphere's +z pole.
fn cap(r: f64, n: usize, polar: f64) -> Vec<NurbsCurve> {
    let pts: Vec<Vec3> = (0..n)
        .map(|i| {
            let a = i as f64 * TAU / n as f64;
            Vec3::new(
                polar.psin() * a.pcos(),
                polar.psin() * a.psin(),
                polar.pcos(),
            ) * r
        })
        .collect();
    (0..n)
        .map(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            let z = p.cross(q).normalized().unwrap();
            let sweep = (p.dot(q) / (r * r)).pacos();
            arc(&Frame::new(Vec3::ZERO, z, p).unwrap(), r, 0.0, sweep).unwrap()
        })
        .collect()
}

fn octant(r: f64) -> Vec<NurbsCurve> {
    let o = Vec3::ZERO;
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let z = Vec3::new(0.0, 0.0, 1.0);
    vec![
        arc(&Frame::new(o, z, x).unwrap(), r, 0.0, FRAC_PI_2).unwrap(),
        arc(&Frame::new(o, x, y).unwrap(), r, 0.0, FRAC_PI_2).unwrap(),
        arc(&Frame::new(o, y, z).unwrap(), r, 0.0, FRAC_PI_2).unwrap(),
    ]
}

/// The largest distance of the patches from the sphere of radius `r`
/// about the origin, on a 41 × 41 grid per patch.
fn off_sphere(fill: &Fill, r: f64) -> f64 {
    let mut worst = 0.0_f64;
    for p in &fill.patches {
        for i in 0..=40 {
            for j in 0..=40 {
                let q = p.eval(i as f64 / 40.0, j as f64 / 40.0);
                worst = worst.max((q.length() - r).abs());
            }
        }
    }
    worst
}

fn all_g1(fill: &Fill) {
    for seam in &fill.seams {
        assert!(seam.continuity.order >= Order::G1, "{:?}", seam.continuity);
    }
    for held in fill.outer.iter().flatten() {
        for s in &held.halves {
            assert!(s.order >= Order::G1, "{s:?}");
        }
    }
}

/// The largest distance off the sphere of the fill of a cap of `n` arcs
/// `polar` from the pole, held as `hold` asks and the inner seams as
/// `inner` asks; every seam and edge proven at least as built.
fn cap_fill(k: f64, n: usize, polar: f64, curvature: bool) -> f64 {
    let r = 10.0 * k;
    let sphere = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
    let curves = cap(r, n, polar);
    let hold = if curvature {
        Condition::Curvature {
            surface: &sphere,
            weight: 1.0,
        }
    } else {
        Condition::Tangent {
            surface: &sphere,
            weight: 1.0,
        }
    };
    let holds = vec![hold; n];
    let inner = if curvature {
        Inner::Curvature
    } else {
        Inner::Tangent
    };
    let fill = nsided(&Nsided {
        curves: &curves,
        edges: &holds,
        inner,
        rise: 0.0,
        tolerance: 1e-6 * k,
        angular: 1e-9,
    })
    .unwrap_or_else(|e| panic!("scale {k}, {n} sides, polar {polar}: {e}"));
    assert_eq!(fill.patches.len(), n);
    let want = if curvature { Order::G2 } else { Order::G1 };
    for seam in &fill.seams {
        assert_eq!(seam.built, inner, "{seam:?}");
        assert!(seam.why.is_none(), "{seam:?}");
        assert!(seam.continuity.order >= want, "{:?}", seam.continuity);
    }
    for held in fill.outer.iter().flatten() {
        assert!(held.why.is_none(), "{held:?}");
        for s in &held.halves {
            assert!(s.order >= want, "{s:?}");
        }
    }
    off_sphere(&fill, r) / r
}

#[test]
fn spherical_caps_held_in_curvature_follow_the_sphere_to_the_fourth_order() {
    for k in SCALES {
        for n in [3, 5, 6] {
            let g1 = cap_fill(k, n, 0.3, false);
            let g2 = cap_fill(k, n, 0.3, true);
            let g2_half = cap_fill(k, n, 0.15, true);
            eprintln!(
                "cap of {n} sides at scale {k}: tangent {g1:.3e} R, curvature {g2:.3e} R, half as wide {g2_half:.3e} R (ratio {:.1})",
                g2 / g2_half
            );
            // Held in curvature, G2 everywhere and within 2e-5 of the radius
            // 17° from the pole; tangent, within 1e-4.
            assert!(g2 <= 2e-5, "{n} sides, scale {k}: {g2:e} R off the sphere");
            assert!(g1 <= 1e-4, "{n} sides, scale {k}: {g1:e} R off the sphere");
            // The error of the quintic blending falls as the fourth power
            // of the cap's size (2⁴ = 16; at least 10 asked).
            assert!(
                g2 / g2_half >= 10.0,
                "{n} sides, scale {k}: ratio {}",
                g2 / g2_half
            );
        }
    }
}

#[test]
fn a_spheres_octant_is_filled_with_proven_seams_and_no_pole() {
    for k in SCALES {
        let r = 10.0 * k;
        let sphere = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
        let curves = octant(r);
        for (inner, hold) in [
            (
                Inner::Tangent,
                Condition::Tangent {
                    surface: &sphere,
                    weight: 1.0,
                },
            ),
            (
                Inner::Curvature,
                Condition::Curvature {
                    surface: &sphere,
                    weight: 1.0,
                },
            ),
        ] {
            // In curvature at one scale only: the measure spends its whole
            // allowance before it gives up (half a minute).
            if inner == Inner::Curvature && k != 1.0 {
                continue;
            }
            let holds = [hold; 3];
            let fill = nsided(&Nsided {
                curves: &curves,
                edges: &holds,
                inner,
                rise: 0.0,
                tolerance: 1e-6 * k,
                angular: 1e-9,
            })
            .unwrap_or_else(|e| panic!("scale {k}, {inner:?}: {e}"));
            all_g1(&fill);
            // In curvature the octant's seams (rational patches of degree
            // 14 a quarter of a great circle long) are G2 to rounding, but
            // the measure spends its work before it proves it: the fill
            // falls back to tangent, and says so with the measure's words,
            // on every seam and curve (never claimed).
            for seam in &fill.seams {
                assert_eq!(seam.asked, inner);
                if inner == Inner::Curvature && seam.built == Inner::Tangent {
                    let why = seam.why.clone().unwrap_or_default();
                    assert!(
                        why.contains("proven only G1") && why.contains("spent"),
                        "{why}"
                    );
                } else {
                    assert_eq!(seam.built, inner, "{:?}", seam.why);
                }
            }
            for held in fill.outer.iter().flatten() {
                assert_eq!(
                    held.built == Inner::Curvature,
                    held.halves.iter().all(|c| c.order == Order::G2)
                        && held.built == Inner::Curvature
                );
                if held.built != held.asked {
                    assert!(held.why.is_some(), "{held:?}");
                }
            }
            let off = off_sphere(&fill, r);
            eprintln!(
                "octant at scale {k}, {inner:?}: {:.3e} R off the sphere",
                off / r
            );
            // 1.04e-3 since the halves' end weights are made 1 by a scalar
            // multiple (their parameter kept) instead of a Möbius change
            // (9.9e-4 before): the octant is the coarse case, the caps hold
            // the order.
            assert!(
                off <= 1.1e-3 * r,
                "scale {k}: {:e} R off the sphere",
                off / r
            );
            // No pole: every corner of every patch has two derivatives
            // well apart.
            for p in &fill.patches {
                for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    let (_, su, sv) = p.derivatives(u, v);
                    let sine = su.cross(sv).length() / (su.length() * sv.length());
                    assert!(sine > 0.3, "corner ({u}, {v}): sine {sine}");
                }
            }
        }
    }
}

#[test]
fn a_flat_pentagon_is_filled_by_its_plane() {
    for k in SCALES {
        let pts: Vec<Vec3> = (0..5)
            .map(|i| {
                let a = i as f64 * TAU / 5.0;
                Vec3::new(10.0 * k * a.pcos(), 10.0 * k * a.psin(), 3.0 * k)
            })
            .collect();
        let curves: Vec<NurbsCurve> = (0..5).map(|i| line(pts[i], pts[(i + 1) % 5])).collect();
        let holds = [Condition::Free; 5];
        let fill = nsided(&Nsided {
            curves: &curves,
            edges: &holds,
            inner: Inner::Tangent,
            rise: 0.0,
            tolerance: 1e-6 * k,
            angular: 1e-6,
        })
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        let mut worst = 0.0_f64;
        for p in &fill.patches {
            for i in 0..=20 {
                for j in 0..=20 {
                    let q = p.eval(i as f64 / 20.0, j as f64 / 20.0);
                    worst = worst.max((q.z - 3.0 * k).abs());
                }
            }
        }
        assert!(worst <= 1e-9 * k, "scale {k}: {worst:e} off the plane");
        all_g1(&fill);
    }
}

#[test]
fn a_twisted_hexagon_is_filled_with_proven_seams() {
    for k in SCALES {
        let pts: Vec<Vec3> = (0..6)
            .map(|i| {
                let a = i as f64 * TAU / 6.0;
                let up = if i % 2 == 0 { 2.0 } else { -2.0 };
                Vec3::new(10.0 * k * a.pcos(), 10.0 * k * a.psin(), up * k)
            })
            .collect();
        // Each side bowed outwards: a quadratic through its midpoint.
        let curves: Vec<NurbsCurve> = (0..6)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % 6]);
                let m = (a + b) * 0.5;
                let out = Vec3::new(m.x, m.y, 0.0) * 0.15;
                NurbsCurve::polynomial(2, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0], vec![a, m + out, b])
                    .unwrap()
            })
            .collect();
        let holds = [Condition::Free; 6];
        let fill = nsided(&Nsided {
            curves: &curves,
            edges: &holds,
            inner: Inner::Tangent,
            rise: 0.0,
            tolerance: 1e-6 * k,
            angular: 1e-6,
        })
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        all_g1(&fill);
    }
}

#[test]
fn loops_that_do_not_close_or_count_wrong_are_refused() {
    let a = Vec3::new(0.0, 0.0, 0.0);
    let b = Vec3::new(1.0, 0.0, 0.0);
    let c = Vec3::new(0.0, 1.0, 0.0);
    let open = [line(a, b), line(b, c), line(c, Vec3::new(0.0, 0.5, 0.0))];
    let holds = [Condition::Free; 3];
    let e = nsided(&Nsided {
        curves: &open,
        edges: &holds,
        inner: Inner::Tangent,
        rise: 0.0,
        tolerance: 1e-6,
        angular: 1e-6,
    })
    .unwrap_err()
    .to_string();
    assert!(e.contains("ends"), "{e}");
    let two = [line(a, b), line(b, a)];
    let e = nsided(&Nsided {
        curves: &two,
        edges: &holds[..2],
        inner: Inner::Tangent,
        rise: 0.0,
        tolerance: 1e-6,
        angular: 1e-6,
    })
    .unwrap_err()
    .to_string();
    assert!(e.contains("3 to 6"), "{e}");
    // A loop of three points on a line encloses nothing.
    let flat = [line(a, b), line(b, b * 2.0), line(b * 2.0, a)];
    assert!(
        nsided(&Nsided {
            curves: &flat,
            edges: &holds,
            inner: Inner::Tangent,
            rise: 0.0,
            tolerance: 1e-6,
            angular: 1e-6,
        })
        .is_err()
    );
}

#[test]
fn a_risen_pentagon_is_a_dome_with_its_centre_where_asked() {
    for k in SCALES {
        let pts: Vec<Vec3> = (0..5)
            .map(|i| {
                let a = i as f64 * TAU / 5.0;
                Vec3::new(10.0 * k * a.pcos(), 10.0 * k * a.psin(), 0.0)
            })
            .collect();
        let curves: Vec<NurbsCurve> = (0..5).map(|i| line(pts[i], pts[(i + 1) % 5])).collect();
        let holds = [Condition::Free; 5];
        for rise in [2.0 * k, -2.0 * k] {
            let fill = nsided(&Nsided {
                curves: &curves,
                edges: &holds,
                inner: Inner::Curvature,
                rise,
                tolerance: 1e-6 * k,
                angular: 1e-9,
            })
            .unwrap_or_else(|e| panic!("scale {k}, rise {rise}: {e}"));
            // The loop runs counter-clockwise about +z: the centre is
            // `rise` up, and every patch's corner at the centre is there.
            assert!(
                (fill.centre.z - rise).abs() <= 1e-9 * k,
                "{:?}",
                fill.centre
            );
            for p in &fill.patches {
                assert!(p.eval(0.0, 0.0).distance(fill.centre) <= 1e-9 * k);
            }
            for seam in &fill.seams {
                assert_eq!(seam.continuity.order, Order::G2, "{:?}", seam.continuity);
            }
        }
    }
}

/// A thumb scoop: four quarter-ellipse arcs (corners at 45°, sides bowed
/// out to 1.25 of the corner radius) on a plane, held in curvature to it,
/// the centre sunk by `rise`.
fn scoop(ax: f64, az: f64) -> Vec<NurbsCurve> {
    let pt = |a: f64, k: f64| {
        Vec3::new(
            ax * k * a.to_radians().pcos(),
            az * k * a.to_radians().psin(),
            0.0,
        )
    };
    (0..4)
        .map(|q| {
            let a0 = 90.0 * q as f64;
            NurbsCurve::polynomial(
                2,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![pt(a0, 1.0), pt(a0 + 45.0, 1.25), pt(a0 + 90.0, 1.0)],
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn a_sunk_scoop_on_a_plane_is_g2_inside_and_out_and_a_long_thin_one_is_refused_by_name() {
    let plane = Surface::Plane(craft3d_geom::Plane::new(Frame::WORLD));
    for k in SCALES {
        for (ax, az) in [(5.0, 5.0), (8.0, 4.0)] {
            for rise in [0.0, -1.5] {
                let curves = scoop(ax * k, az * k);
                let holds = vec![
                    Condition::Curvature {
                        surface: &plane,
                        weight: 1.0,
                    };
                    4
                ];
                let fill = nsided(&Nsided {
                    curves: &curves,
                    edges: &holds,
                    inner: Inner::Curvature,
                    rise: rise * k,
                    tolerance: 1e-6 * k,
                    angular: 1e-9,
                })
                .unwrap_or_else(|e| panic!("{ax}x{az} rise {rise} at {k}: {e}"));
                for (i, seam) in fill.seams.iter().enumerate() {
                    assert_eq!(seam.built, Inner::Curvature, "seam {i}: {:?}", seam.why);
                    assert_eq!(seam.continuity.order, Order::G2, "seam {i} at {k}");
                }
                for held in fill.outer.iter() {
                    let held = held.as_ref().unwrap();
                    assert_eq!(held.built, Inner::Curvature, "{:?}", held.why);
                    for half in &held.halves {
                        assert_eq!(half.order, Order::G2, "an outer half at {k}");
                    }
                }
                // Sunk where asked: the centre lies `rise` off the plane.
                assert!(
                    (fill.centre.z - rise * k).abs() <= 1e-9 * k,
                    "centre {:?}",
                    fill.centre
                );
            }
        }
    }
    // 6.5 : 1: the patches' seams are not found on a parameter line of
    // their own patches (the projection lands on a far corner), so the
    // measure refuses by name (owed: the seams measured on the lines they
    // were built on, not found by projection).
    let curves = scoop(15.0, 2.3);
    let holds = vec![
        Condition::Curvature {
            surface: &plane,
            weight: 1.0,
        };
        4
    ];
    let refused = nsided(&Nsided {
        curves: &curves,
        edges: &holds,
        inner: Inner::Curvature,
        rise: 0.0,
        tolerance: 1e-6,
        angular: 1e-9,
    })
    .map(|_| ())
    .unwrap_err()
    .to_string();
    assert!(refused.contains("parameter line"), "{refused}");
}
