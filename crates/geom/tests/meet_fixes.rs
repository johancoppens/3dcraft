//! Regression tests for defects of `meet_surfaces` found by the booleans.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_geom::{NurbsSurface, Plane, Sphere, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

fn everywhere() -> Bbox3 {
    Bbox3::from_points([Vec3::new(-9.0, -9.0, -9.0), Vec3::new(9.0, 9.0, 9.0)])
}

#[test]
fn spheres_apart_by_the_tolerance_to_the_last_bit_touch_or_miss_but_do_not_fail() {
    // Found as `near/sphere-sphere/017` of the boolean corpus: radii 1.375
    // and 1, centres 2.375 + 1e-6 apart, where 2.000001 - (-0.375) is a
    // hair over the tolerance in floating point. It was neither "apart"
    // nor "touching" and ended as a circle of radius zero, an error.
    let tol = Tolerance::default();
    let a = Surface::Sphere(
        Sphere::new(
            Frame::WORLD.at(Vec3::new(1.5, -0.375, 1.375)).unwrap(),
            1.375,
        )
        .unwrap(),
    );
    for gap in [0.0, 0.5e-6, 1.0e-6, 1.0000001e-6, 1.5e-6, -1.0e-6] {
        let b = Surface::Sphere(
            Sphere::new(
                Frame::WORLD.at(Vec3::new(1.5, 2.0 + gap, 1.375)).unwrap(),
                1.0,
            )
            .unwrap(),
        );
        match meet_surfaces(&a, &b, &everywhere(), &tol) {
            Ok(SurfaceMeeting::None) => assert!(gap > 0.9e-6, "{gap}"),
            Ok(SurfaceMeeting::Meets(meeting)) => {
                // A touch, or (overlapping by a tolerance) a small circle.
                assert!(meeting.curves.len() + meeting.points.len() == 1, "{gap}");
            }
            other => panic!("{gap}: {other:?}"),
        }
    }
}

/// The pieces of a curve that a box cuts are held to the tolerance (the
/// lamp's finding B1, 2026-10-15). The outer wall of a lampshade turned
/// from a spline (a rational surface with a pole at its top) and the end
/// cap of a lofted cell meet in a closed curve, which the box (the one
/// the ten cells of the lamp gave this pair, inlined) cuts. The span from
/// the last point inside to the point on the box was a new cubic that
/// was never refined: "an intersection curve could not be fitted within
/// the tolerance" (1.05e-6 where the target is 0.5e-6).
#[test]
fn a_curve_cut_by_the_box_stays_within_the_tolerance_along_the_cut_piece() {
    let tol = Tolerance::default();
    let cap = Surface::Plane(Plane::new(
        Frame::new(
            Vec3::new(-101.30507340959298, -111.29355510700076, 141.5504168692775),
            Vec3::new(0.6691084494494924, 0.7350812312260296, -0.10931361477199582),
            Vec3::new(0.5964115907684737, -0.4433814354168595, 0.6691084494494924),
        )
        .unwrap(),
    ));
    let wall = wall();
    let within = Bbox3::from_points([
        Vec3::new(-168.04316601773542, -168.04316601773542, 104.38877107830017),
        Vec3::new(167.06945264388725, 168.0431660177354, 198.75929163002164),
    ]);
    let met = meet_surfaces(&wall, &cap, &within, &tol).unwrap();
    let SurfaceMeeting::Meets(meeting) = met else {
        panic!("the wall and the cap meet: {met:?}");
    };
    let cut = meeting.curves.iter().filter(|c| !c.closed).count();
    assert!(cut > 0, "the box cuts the curve");
    for curve in &meeting.curves {
        assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
        let (t0, t1) = curve.range;
        for k in 0..=2000 {
            let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(k) / 2000.0);
            assert!(within.contains(p, tol.linear), "{p:?} outside the box");
            for surface in [&wall, &cap] {
                let (u, v) = surface.project(p).unwrap();
                let off = surface.eval(u, v).distance(p);
                assert!(off <= tol.linear, "{off} off a surface at {p:?}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn nurbs(
    degree_u: usize,
    degree_v: usize,
    knots_u: &[f64],
    knots_v: &[f64],
    count_u: usize,
    count_v: usize,
    ctrl: &[[f64; 3]],
    weights: &[f64],
) -> Surface {
    Surface::Nurbs(
        NurbsSurface::new(
            degree_u,
            degree_v,
            knots_u.to_vec(),
            knots_v.to_vec(),
            count_u,
            count_v,
            ctrl.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect(),
            weights.to_vec(),
        )
        .unwrap(),
    )
}

/// The lampshade's outer wall: its profile, a cubic spline from the axis
/// at the top to the rim, turned a whole turn (the kernel's revolve).
// (The weights are written as the kernel computed them: 1/√2 where a
// quarter turn has its middle row.)
#[allow(clippy::approx_constant)]
fn wall() -> Surface {
    nurbs(
        2,
        3,
        &[0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        &[0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0],
        5,
        7,
        &[
            [0.0, 0.0, 250.0],
            [-60.0, 7.34788079488412e-15, 250.0],
            [-128.0, 1.567547902908612e-14, 238.0],
            [-162.0, 1.9839278146187122e-14, 175.0],
            [-160.0, 1.9594348786357652e-14, 95.0],
            [-134.0, 1.6410267108574532e-14, 35.0],
            [-120.0, 1.469576158976824e-14, 0.0],
            [0.0, 0.0, 250.0],
            [-60.000000000000014, -59.99999999999999, 250.0],
            [-128.00000000000003, -127.99999999999999, 238.0],
            [-162.00000000000003, -161.99999999999997, 175.0],
            [-160.00000000000003, -159.99999999999997, 95.0],
            [-134.00000000000003, -133.99999999999997, 35.0],
            [-120.00000000000003, -119.99999999999999, 0.0],
            [0.0, 0.0, 250.0],
            [-1.1021821192326178e-14, -60.0, 250.0],
            [-2.351321854362918e-14, -128.0, 238.0],
            [-2.975891721928068e-14, -162.0, 175.0],
            [-2.9391523179536473e-14, -160.0, 95.0],
            [-2.4615400662861796e-14, -134.0, 35.0],
            [-2.2043642384652355e-14, -120.0, 0.0],
            [0.0, 0.0, 250.0],
            [59.99999999999998, -60.000000000000014, 250.0],
            [127.99999999999996, -128.00000000000003, 238.0],
            [161.99999999999994, -162.00000000000003, 175.0],
            [159.99999999999994, -160.00000000000003, 95.0],
            [133.99999999999994, -134.00000000000003, 35.0],
            [119.99999999999996, -120.00000000000003, 0.0],
            [0.0, 0.0, 250.0],
            [60.0, -1.469576158976824e-14, 250.0],
            [128.0, -3.135095805817224e-14, 238.0],
            [162.0, -3.9678556292374245e-14, 175.0],
            [160.0, -3.9188697572715304e-14, 95.0],
            [134.0, -3.2820534217149064e-14, 35.0],
            [120.0, -2.939152317953648e-14, 0.0],
        ],
        &[
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            0.7071067811865476,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
            1.0,
        ],
    )
}

/// A closed curve much smaller than the box is gone around once (the
/// turbofan's finding F10, 2026-10-15). A blade's leading edge rounded
/// to 0.1 mm stands on a disc's rim of radius 27.6: their cylinders meet
/// in a loop 0.63 mm round, asked inside the box of the two bodies (81 mm
/// across). The marcher's first step is a share of the box (0.63 mm),
/// and it went once around the loop to just past where it began, with
/// the tangent it began with: the step was taken, the march went on
/// round again, and the curve was the loop twice. A step that ends less
/// than half as far as it was asked is now refused, as one that ends more
/// than twice as far was. Held: in boxes from the loop's size to a
/// thousand times the disc's, upright and tilted, at three scales, the
/// curve is closed, goes once around the small cylinder's axis (the
/// angle it turns about it is 2π) and lies on both surfaces.
#[test]
fn a_small_closed_curve_in_a_large_box_is_gone_around_once() {
    use craft3d_geom::Cylinder;
    use craft3d_math::Transform;
    use std::f64::consts::{PI, TAU};
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for tilted in [false, true] {
            let place = if tilted {
                Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
            } else {
                Transform::IDENTITY
            };
            let disc = Frame::new(Vec3::new(0.0, 113.5, 0.0) * k, Vec3::Y, Vec3::X).unwrap();
            let lean = Vec3::new(0.9344942231690109, -0.355978295495311, 0.0);
            let edge = Frame::new(
                Vec3::new(-1.5084323029781646, 120.04660057552549, -26.41745608814259) * k,
                -Vec3::Z,
                lean,
            )
            .unwrap();
            let big = Surface::Cylinder(Cylinder::new(disc, 27.5789 * k).unwrap())
                .transformed(&place)
                .unwrap();
            let small = Surface::Cylinder(Cylinder::new(edge, 0.1 * k).unwrap())
                .transformed(&place)
                .unwrap();
            let (axis, across) = (place.vector(-Vec3::Z), place.vector(lean));
            let side = axis.cross(across);
            let centre = place.point(edge.origin());
            let middle = place.point(Vec3::new(-1.51, 120.05, -27.54) * k);
            // The box of the two bodies in the turbofan, and boxes about
            // the loop from its size to a thousand times the disc's.
            let found = (
                Vec3::new(-28.594371140143355, 115.0436492847657, -28.594371140143355),
                Vec3::new(28.594371140143355, 121.16207171566876, 28.594371140143355),
            );
            let mut boxes = vec![(found.0 * k, found.1 * k)];
            for reach in [0.3, 1.0, 3.0, 10.0, 30.0, 100.0, 300.0, 3e4] {
                let half = Vec3::new(1.0, 1.0, 1.0) * reach;
                let at = Vec3::new(-1.51, 120.05, -27.54);
                boxes.push(((at - half) * k, (at + half) * k));
            }
            for (n, (lo, hi)) in boxes.into_iter().enumerate() {
                let corners = (0..8).map(|c| {
                    place.point(Vec3::new(
                        if c & 1 == 0 { lo.x } else { hi.x },
                        if c & 2 == 0 { lo.y } else { hi.y },
                        if c & 4 == 0 { lo.z } else { hi.z },
                    ))
                });
                let within = Bbox3::from_points(corners);
                let what = format!("at {k}, tilted {tilted}, box {n}");
                let Ok(SurfaceMeeting::Meets(meeting)) = meet_surfaces(&big, &small, &within, &tol)
                else {
                    panic!("{what}: no curve");
                };
                let near: Vec<_> = meeting
                    .curves
                    .iter()
                    .filter(|c| c.curve.eval(c.range.0).distance(middle) < k)
                    .collect();
                assert_eq!(near.len(), 1, "{what}");
                let curve = near[0];
                assert!(curve.closed, "{what}");
                let (t0, t1) = curve.range;
                let angle = |p: Vec3| {
                    let d = p - centre;
                    d.dot(side).atan2(d.dot(across))
                };
                let (mut turned, mut last) = (0.0, angle(curve.curve.eval(t0)));
                for i in 1..=2000 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 2000.0);
                    for s in [&big, &small] {
                        let (u, v) = s.project(p).unwrap();
                        assert!(
                            s.eval(u, v).distance(p) <= tol.linear,
                            "{what}: off a surface"
                        );
                    }
                    let now = angle(p);
                    let mut d = now - last;
                    if d > PI {
                        d -= TAU;
                    } else if d < -PI {
                        d += TAU;
                    }
                    turned += d;
                    last = now;
                }
                assert!(
                    (turned.abs() - TAU).abs() < 1e-6,
                    "{what}: the curve turns {turned} about the small cylinder's axis"
                );
            }
        }
    }
}

/// Only the part of a freeform chart that can be inside the box is
/// looked at (the bike's finding B9, 2026-10-15). A top tube lofted
/// through an oval and two circles less its core, the same loft 1.2 mm
/// inside it: the tube's tapered face (a ruled rational patch 325 mm
/// long) and the core's cylinder were asked in the 12 mm box where both
/// faces are, at the oval where the taper begins. The cylinder runs on
/// inside the taper and crosses it 300 mm further along at a shallow
/// angle; the patch was walked whole, and the cells along that crossing
/// ran out ("tangent or nearly coincident over too large an area"). A
/// cell of a freeform chart whose image lies further from the box than
/// the chart moves across the cell is now left out, and the sides of the
/// cells left in whose image may reach out of the box are searched along
/// their length, as the chart's borders are. Held, upright and tilted, at
/// three scales: nothing in the box (where every sampled point of the
/// patch is more than a millimetre from the cylinder), and in a box about
/// the crossing the curves found lie on both surfaces.
#[test]
fn only_the_part_of_a_freeform_chart_in_the_box_is_walked() {
    use craft3d_geom::Cylinder;
    use craft3d_math::Transform;
    use std::f64::consts::FRAC_1_SQRT_2;
    let ctrl = [
        [236.43490944998672, 3.330106906, 562.0562330784912],
        [-88.47870224995935, 10.960155108, 520.7350331902426],
        [
            236.60164237822977,
            1.7763568394002505e-14,
            560.9598888682966,
        ],
        [-86.8308205049187, 5.861977570020827e-14, 509.89946741148503],
        [236.43490944998672, -3.330106906, 562.0562330784912],
        [-88.47870224995935, -10.960155108, 520.7350331902426],
    ];
    let weights = [1.0, 1.0, 0.9487760340809234, FRAC_1_SQRT_2, 1.0, 1.0];
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for tilted in [false, true] {
            let place = if tilted {
                Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
            } else {
                Transform::IDENTITY
            };
            let scaled: Vec<[f64; 3]> = ctrl
                .iter()
                .map(|p| {
                    let q = place.point(Vec3::new(p[0], p[1], p[2]) * k);
                    [q.x, q.y, q.z]
                })
                .collect();
            let taper = nurbs(
                2,
                1,
                &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                &[0.0, 0.0, 1.0, 1.0],
                3,
                2,
                &scaled,
                &weights,
            );
            let axis = place.vector(Vec3::new(-0.9886325213304246, 0.0, -0.1503520461046269));
            let across = place.vector(Vec3::new(
                0.1413166234899125,
                0.34143637192069665,
                -0.9292205420969997,
            ));
            let origin = place.point(Vec3::new(411.40948929406585, 0.0, 599.3965842148483) * k);
            let radius = 9.547999999710598 * k;
            let core = Surface::Cylinder(
                Cylinder::new(Frame::new(origin, axis, across).unwrap(), radius).unwrap(),
            );
            let from_axis = |p: Vec3| {
                let d = p - origin;
                (d - axis * d.dot(axis)).length()
            };
            let boxed = |lo: Vec3, hi: Vec3| {
                Bbox3::from_points((0..8).map(|c| {
                    place.point(
                        Vec3::new(
                            if c & 1 == 0 { lo.x } else { hi.x },
                            if c & 2 == 0 { lo.y } else { hi.y },
                            if c & 4 == 0 { lo.z } else { hi.z },
                        ) * k,
                    )
                }))
            };
            let what = format!("at {k}, tilted {tilted}");
            // The box the boolean asked in.
            let within = boxed(
                Vec3::new(229.00562451389766, -13.946774149715337, 558.7055076669526),
                Vec3::new(241.0004165282345, 13.946774149715337, 586.761617485485),
            );
            for i in 0..=40 {
                for j in 0..=40 {
                    let p = taper.eval(f64::from(i) / 40.0, f64::from(j) / 40.0);
                    if within.contains(p, 0.0) {
                        assert!((from_axis(p) - radius).abs() > 1.0 * k, "{what}");
                    }
                }
            }
            match meet_surfaces(&taper, &core, &within, &tol) {
                Ok(SurfaceMeeting::None) => {}
                Ok(SurfaceMeeting::Meets(m)) if m.curves.is_empty() && m.points.is_empty() => {}
                other => panic!("{what}: {other:?}"),
            }
            // Where the cylinder comes out through the taper.
            let mut crossing = None;
            for j in 0..400 {
                let (v0, v1) = (f64::from(j) / 400.0, f64::from(j + 1) / 400.0);
                let (a, b) = (taper.eval(0.5, v0), taper.eval(0.5, v1));
                if (from_axis(a) - radius) * (from_axis(b) - radius) < 0.0 {
                    crossing = Some(a);
                }
            }
            let crossing = crossing.unwrap_or_else(|| panic!("{what}: no crossing"));
            let reach = Vec3::new(8.0, 8.0, 8.0) * k;
            let there = Bbox3::from_points([crossing - reach, crossing + reach]);
            let Ok(SurfaceMeeting::Meets(m)) = meet_surfaces(&taper, &core, &there, &tol) else {
                panic!("{what}: no curve where they cross");
            };
            assert!(!m.curves.is_empty(), "{what}");
            for curve in &m.curves {
                let (t0, t1) = curve.range;
                for i in 0..=200 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 200.0);
                    assert!(
                        (from_axis(p) - radius).abs() <= tol.linear,
                        "{what}: off the cylinder"
                    );
                    let (u, v) = taper.project(p).unwrap();
                    assert!(
                        taper.eval(u, v).distance(p) <= tol.linear,
                        "{what}: off the taper"
                    );
                }
            }
        }
    }
}

/// Two traces are one branch only where one lies on the other's curve,
/// not near its chords (the turbofan's benchmark, 2026-10-15: the cones of
/// the rounded edges of two neighbouring blades of a row, a pattern about
/// the engine's axis, in the box of the two faces). Two of their curves
/// pass close to each other; while branches were stopped at the box
/// (tried and dropped the same day), the traces of the two, tested
/// against chords that stray from them, were taken for one, joined
/// across 25 mm with opposite tangents, and the curve was "not fitted".
/// Upright and turned, at three scales: every curve within the
/// tolerance of both cones.
#[test]
fn the_cones_of_two_neighbouring_blades_meet_in_curves_on_both() {
    use craft3d_geom::Cone;
    use craft3d_math::Transform;
    let cone = |origin: [f64; 3], z: [f64; 3], x: [f64; 3], k: f64, place: &Transform| {
        let frame = Frame::new(
            Vec3::new(origin[0], origin[1], origin[2]) * k,
            Vec3::new(z[0], z[1], z[2]),
            Vec3::new(x[0], x[1], x[2]),
        )
        .unwrap();
        Surface::Cone(Cone::new(frame, 6.15207092695502 * k, -0.7853981633974495).unwrap())
            .transformed(place)
            .unwrap()
    };
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for tilted in [false, true] {
            let place = if tilted {
                Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
            } else {
                Transform::IDENTITY
            };
            let a = cone(
                [45.33251775640262, 209.16400209858972, -26.891379727470436],
                [0.8910065241883679, 0.0, -0.4539904997395467],
                [
                    0.013530354188930841,
                    0.9995557869880324,
                    0.026554815274401296,
                ],
                k,
                &place,
            );
            let b = cone(
                [40.56766063596038, 209.1640020985897, -33.651870383488614],
                [0.8090169943749477, 0.0, -0.5877852522924731],
                [
                    0.01751786139822269,
                    0.9995557869880323,
                    0.024111267713833588,
                ],
                k,
                &place,
            );
            let (lo, hi) = (
                Vec3::new(27.278797191516883, 191.49909978317802, -37.76971739953638) * k,
                Vec3::new(44.939184099973296, 226.82890441400139, -12.851210762358724) * k,
            );
            let within = Bbox3::from_points((0..8).map(|c| {
                place.point(Vec3::new(
                    if c & 1 == 0 { lo.x } else { hi.x },
                    if c & 2 == 0 { lo.y } else { hi.y },
                    if c & 4 == 0 { lo.z } else { hi.z },
                ))
            }));
            let what = format!("at {k}, tilted {tilted}");
            let Ok(SurfaceMeeting::Meets(meeting)) = meet_surfaces(&a, &b, &within, &tol) else {
                panic!("{what}: the cones meet");
            };
            assert!(!meeting.curves.is_empty(), "{what}");
            for curve in &meeting.curves {
                let (t0, t1) = curve.range;
                for i in 0..=400 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 400.0);
                    for s in [&a, &b] {
                        let (u, v) = s.project(p).unwrap();
                        assert!(s.eval(u, v).distance(p) <= tol.linear, "{what}: off a cone");
                    }
                }
            }
        }
    }
}

/// A curve that leaves the box and comes back into it through cells left
/// out (only the part of a chart that can be inside the box is walked,
/// 2026-10-15): a flat spline patch 200 wide against a cylinder of radius
/// 1.2 standing on it, in the box |x| ≤ 10, |y| ≤ 1. About its axis
/// their circle leaves the box across y = ±1 and comes back (two arcs,
/// 4 asin(1/1.2) of it); moved to y = 2.19 it dips into the box by a
/// hundredth (2 acos(1.19/1.2) of it). Nearly all of the patch's cells
/// are left out. What the curves found turn through is that (closed form, and the crossings of the circle with the
/// box found by bisection agree with it), on both surfaces. Upright and
/// turned by quarter turns (the box stays the slab), at three scales.
#[test]
fn a_curve_that_leaves_the_box_and_comes_back_is_found_both_times() {
    use craft3d_geom::Cylinder;
    use craft3d_math::Transform;
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for (centre, closed_form) in [
            (0.0, 4.0 * (1.0_f64 / 1.2).asin()),
            (2.19, 2.0 * (1.19_f64 / 1.2).acos()),
        ] {
            for tilted in [false, true] {
                // (Turned by quarter turns, so that the box stays the slab.)
                let place = if tilted {
                    Transform::rotation(Vec3::Z, std::f64::consts::FRAC_PI_2)
                        .unwrap()
                        .then(&Transform::rotation(Vec3::Y, std::f64::consts::PI).unwrap())
                        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
                } else {
                    Transform::IDENTITY
                };
                let corner = |x: f64, y: f64| {
                    let p = place.point(Vec3::new(x, y, 0.0) * k);
                    [p.x, p.y, p.z]
                };
                let flat = nurbs(
                    1,
                    1,
                    &[0.0, 0.0, 1.0, 1.0],
                    &[0.0, 0.0, 1.0, 1.0],
                    2,
                    2,
                    &[
                        corner(-100.0, -100.0),
                        corner(-100.0, 100.0),
                        corner(100.0, -100.0),
                        corner(100.0, 100.0),
                    ],
                    &[1.0; 4],
                );
                let axis = Frame::new(
                    place.point(Vec3::new(0.0, centre, 0.0) * k),
                    place.vector(Vec3::Z),
                    place.vector(Vec3::X),
                )
                .unwrap();
                let radius = 1.2 * k;
                let rod = Surface::Cylinder(Cylinder::new(axis, radius).unwrap());
                let within = Bbox3::from_points((0..8).map(|c| {
                    place.point(
                        Vec3::new(
                            if c & 1 == 0 { -10.0 } else { 10.0 },
                            if c & 2 == 0 { -1.0 } else { 1.0 },
                            if c & 4 == 0 { -1.0 } else { 1.0 },
                        ) * k,
                    )
                }));
                let what = format!("at {k}, centre {centre}, tilted {tilted}");
                let Ok(SurfaceMeeting::Meets(meeting)) = meet_surfaces(&flat, &rod, &within, &tol)
                else {
                    panic!("{what}: no curve");
                };
                // The arcs the box keeps, by the angle each turns about the
                // axis (the box is a slab tilted with the rest, so in the
                // plane of the patch it keeps |y| <= 1 still).
                let mut turned = 0.0;
                for curve in &meeting.curves {
                    let (t0, t1) = curve.range;
                    let angle = |p: Vec3| {
                        let (x, y, _) = axis.local(p);
                        y.atan2(x)
                    };
                    let mut last = angle(curve.curve.eval(t0));
                    for i in 1..=400 {
                        let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 400.0);
                        for s in [&flat, &rod] {
                            let (u, v) = s.project(p).unwrap();
                            assert!(
                                s.eval(u, v).distance(p) <= tol.linear,
                                "{what}: off a surface"
                            );
                        }
                        let now = angle(p);
                        let d = (now - last + std::f64::consts::PI)
                            .rem_euclid(std::f64::consts::TAU)
                            - std::f64::consts::PI;
                        turned += d.abs();
                        last = now;
                    }
                }
                // What of the circle is in the box, by its crossings of the
                // box, each found by bisection (upright: 4 asin(1/1.2)).
                let on_circle = |a: f64| axis.point(radius * a.cos(), radius * a.sin(), 0.0);
                let inside = |a: f64| within.contains(on_circle(a), 0.0);
                let steps = 4096;
                let mut expected = 0.0;
                let mut from: Option<f64> = None;
                let start = (0..steps)
                    .map(|i| std::f64::consts::TAU * f64::from(i) / f64::from(steps))
                    .find(|&a| !inside(a))
                    .unwrap();
                let cross = |lo: f64, hi: f64| {
                    let (mut lo, mut hi) = (lo, hi);
                    let was = inside(lo);
                    for _ in 0..80 {
                        let m = 0.5 * (lo + hi);
                        if inside(m) == was {
                            lo = m;
                        } else {
                            hi = m;
                        }
                    }
                    0.5 * (lo + hi)
                };
                for i in 0..steps {
                    let step = std::f64::consts::TAU / f64::from(steps);
                    let (a, b) = (start + step * f64::from(i), start + step * f64::from(i + 1));
                    if inside(a) != inside(b) {
                        let c = cross(a, b);
                        match from.take() {
                            None => from = Some(c),
                            Some(f) => expected += c - f,
                        }
                    }
                }
                assert!(
                    (expected - closed_form).abs() < 1e-9,
                    "{expected} {closed_form}"
                );
                assert!(
                    (turned - expected).abs() <= 1e-6,
                    "{what}: the arcs turn {turned}, not {expected} ({} curves)",
                    meeting.curves.len()
                );
            }
        }
    }
}

/// A plane drawn as a quadratic NURBS surface of two spans along u whose
/// weights go up by `skew` from one control point to the next: each span a
/// Möbius reparametrisation of itself, so that where they meet (u = 1, the
/// line x = 2) the surface moves `skew²` times faster just after the knot
/// than just before it.
fn skewed_plane(skew: f64) -> Surface {
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..5 {
        for y in [-2.0, 2.0] {
            ctrl.push(Vec3::new(i as f64, y, 0.0));
            weights.push(skew.powi(i));
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0],
            vec![0.0, 0.0, 1.0, 1.0],
            5,
            2,
            ctrl,
            weights,
        )
        .unwrap(),
    )
}

#[test]
fn a_circle_across_a_knot_where_the_speed_jumps_is_marched_once() {
    // Review of 2026-10-17: where the step across such a knot is scaled to
    // the length it went (`Marcher::rescaled`), a loop that closes during
    // that step must close there, not be marched around a second time,
    // and the step must stay on its own branch. A ball on the knot line
    // meets the plane in one circle, crossing the knot twice.
    let tol = Tolerance::default();
    for skew in [1.0, 10.0, 31.6] {
        for centre in [Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.3, 0.4, 0.2)] {
            let radius = 0.5_f64;
            let ball =
                Surface::Sphere(Sphere::new(Frame::WORLD.at(centre).unwrap(), radius).unwrap());
            let what = format!("skew {skew}, ball at {centre:?}");
            let Ok(SurfaceMeeting::Meets(meeting)) =
                meet_surfaces(&skewed_plane(skew), &ball, &everywhere(), &tol)
            else {
                panic!("{what}: no meeting");
            };
            assert_eq!(meeting.curves.len(), 1, "{what}");
            let curve = &meeting.curves[0];
            assert!(curve.closed, "{what}");
            // Its length is the circle's, once around.
            let (t0, t1) = curve.range;
            let mut length = 0.0;
            let mut last = curve.curve.eval(t0);
            for k in 1..=4000 {
                let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(k) / 4000.0);
                length += p.distance(last);
                last = p;
                let off_ball = p.distance(centre) - radius;
                assert!(off_ball.abs() <= 1e-5 && p.z.abs() <= 1e-5, "{what}: {p:?}");
            }
            let circle = std::f64::consts::TAU * (radius * radius - centre.z * centre.z).sqrt();
            assert!(
                (length - circle).abs() <= 1e-4 * circle,
                "{what}: {length} for {circle}"
            );
        }
    }
}
