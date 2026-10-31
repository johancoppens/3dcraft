//! The numeric marcher: against the closed forms it can be forced onto,
//! and on the classic curves that have no closed form here.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::*;
use craft3d_geom::intersect::{Meeting, meet_surfaces, meet_surfaces_numeric};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
// The time bounds are on the work of this thread, which a busy machine
// does not stretch.
use craft3d_testkit::cost::WorkClock;

/// The length of a meeting curve (a polyline of 2000 chords).
fn length(curve: &craft3d_geom::intersect::MeetingCurve) -> f64 {
    samples(curve, 2000)
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .sum()
}

/// The marcher and the closed form agree: the same number of curves, each
/// numeric curve along an exact one and as long.
fn same_as_exact(a: &Surface, b: &Surface, within: &Bbox3, t: &Tolerance) -> (Meeting, Meeting) {
    let exact = meeting(meet_surfaces(a, b, within, t).unwrap());
    let clock = std::time::Instant::now();
    let numeric = meeting(meet_surfaces_numeric(a, b, within, t).unwrap());
    eprintln!(
        "{:>9} / {:<9} {:>4} curves {:>6} ctrl {:>8.1} ms",
        a.kind(),
        b.kind(),
        numeric.curves.len(),
        numeric
            .curves
            .iter()
            .map(|c| match &c.curve {
                Curve3::Nurbs(n) => n.control_points().len(),
                _ => 0,
            })
            .sum::<usize>(),
        clock.elapsed().as_secs_f64() * 1e3
    );
    check_meeting(&numeric, a, b, within, t);
    assert_eq!(
        numeric.curves.len(),
        exact.curves.len(),
        "{} against {}: {numeric:?}",
        a.kind(),
        b.kind()
    );
    assert!(numeric.curves.iter().all(|c| !c.exact && !c.tangent));
    let scale = within.diagonal();
    let (numeric_lines, exact_lines) = (Lines::of(&numeric, 4000), Lines::of(&exact, 4000));
    for curve in &numeric.curves {
        for p in samples(curve, 60) {
            let off = exact_lines.distance(p);
            assert!(
                off <= 2.0 * t.linear + 1e-6 * scale,
                "{off} from the exact curve"
            );
        }
    }
    for curve in &exact.curves {
        for p in samples(curve, 60) {
            let off = numeric_lines.distance(p);
            assert!(
                off <= 2.0 * t.linear + 1e-6 * scale,
                "{off} from the marched curve"
            );
        }
    }
    let total = |m: &Meeting| m.curves.iter().map(length).sum::<f64>();
    assert!((total(&numeric) - total(&exact)).abs() <= 1e-4 * scale);
    let closed = |m: &Meeting| m.curves.iter().filter(|c| c.closed).count();
    assert_eq!(closed(&numeric), closed(&exact));
    (numeric, exact)
}

#[test]
fn the_marcher_reproduces_the_closed_forms() {
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let within = cube(f.origin(), 12.0 * s);
        let n = f.vector(0.2, 0.5, 1.0);
        let cut = plane(Frame::new(f.point(0.1 * s, 0.2 * s, 0.6 * s), n, f.x()).unwrap());
        // Circle, ellipse, lines.
        same_as_exact(&cut, &sphere(f, 3.0 * s), &within, &t);
        same_as_exact(&cut, &cylinder(f, 2.0 * s), &within, &t);
        let along = plane(Frame::new(f.point(s, 0.0, 0.0), f.x(), f.z()).unwrap());
        same_as_exact(&along, &cylinder(f, 2.0 * s), &within, &t);
        same_as_exact(&cut, &along, &within, &t);
        // Conics of a cone, and circles about a shared axis.
        same_as_exact(&cut, &cone(f, 1.5 * s, 0.4), &within, &t);
        same_as_exact(&along, &cone(f, 1.5 * s, 0.4), &within, &t);
        let ball = sphere(f.at(f.point(0.0, 0.0, s)).unwrap(), 3.0 * s);
        same_as_exact(&cylinder(f, 2.0 * s), &ball, &within, &t);
        same_as_exact(&cone(f, 1.5 * s, 0.4), &ball, &within, &t);
        same_as_exact(
            &sphere(f, 3.0 * s),
            &sphere(f.at(f.point(s, 2.0 * s, s)).unwrap(), 2.5 * s),
            &within,
            &t,
        );
        // The torus: circles about its axis and around its tube.
        let ring = torus(f, 4.0 * s, s);
        same_as_exact(
            &plane(f.at(f.point(0.0, 0.0, 0.5 * s)).unwrap()),
            &ring,
            &within,
            &t,
        );
        same_as_exact(&along_axis(&f), &ring, &within, &t);
        same_as_exact(&cylinder(f, 4.0 * s), &ring, &within, &t);
        same_as_exact(&sphere(f, 4.5 * s), &ring, &within, &t);
    }
}

fn along_axis(f: &Frame) -> Surface {
    plane(Frame::new(f.origin(), f.x(), f.z()).unwrap())
}

#[test]
fn equal_cylinders_at_right_angles_by_marching() {
    // Two ellipses through the two points where the cylinders touch. The
    // marcher stops at those points: four arcs, as the closed form gives.
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let within = cube(f.origin(), 12.0 * s);
        let a = cylinder(f, 2.0 * s);
        let b = cylinder(Frame::new(f.origin(), f.x(), f.z()).unwrap(), 2.0 * s);
        let (numeric, _) = same_as_exact(&a, &b, &within, &t);
        assert_eq!(numeric.curves.len(), 4);
        assert!(numeric.points.is_empty());
        let poles = [f.point(0.0, 2.0 * s, 0.0), f.point(0.0, -2.0 * s, 0.0)];
        for curve in &numeric.curves {
            for p in [start(curve), end(curve)] {
                assert!(poles.iter().any(|q| q.distance(p) <= 2.0 * t.linear));
            }
            assert!(start(curve).distance(end(curve)) > s);
        }
    }
}

#[test]
fn clipped_marched_curves_end_on_the_box() {
    // Crossing cylinders in a box that cuts both loops.
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let big = cylinder(f, 2.0 * s);
        let small = cylinder(
            Frame::new(f.point(0.0, 0.3 * s, 0.0), f.x(), f.z()).unwrap(),
            s,
        );
        let whole = meeting(meet_surfaces(&big, &small, &cube(f.origin(), 12.0 * s), &t).unwrap());
        let whole_lines = Lines::of(&whole, 4000);
        let (center, half) = (f.point(1.8 * s, 0.2 * s, 0.4 * s), 0.9 * s);
        let within = cube(center, half);
        let m = meeting(meet_surfaces(&big, &small, &within, &t).unwrap());
        assert_eq!(
            m,
            meeting(meet_surfaces(&small, &big, &within, &t).unwrap())
        );
        check_meeting(&m, &big, &small, &within, &t);
        assert!(!m.curves.is_empty() && m.curves.iter().all(|c| !c.closed));
        let shrunk = cube(center, half - 1e-4 * s);
        for curve in &m.curves {
            assert!(!shrunk.contains(start(curve), 0.0), "starts inside the box");
            assert!(!shrunk.contains(end(curve), 0.0), "ends inside the box");
        }
        // Every part of the whole curves inside the box is there.
        let lines = Lines::of(&m, 4000);
        let mut inside = 0;
        for curve in &whole.curves {
            for p in samples(curve, 2000) {
                if shrunk.contains(p, 0.0) {
                    inside += 1;
                    assert!(lines.distance(p) <= 1e-5 * s);
                }
            }
        }
        assert!(inside > 50);
        for curve in &m.curves {
            for p in samples(curve, 200) {
                assert!(whole_lines.distance(p) <= 1e-5 * s);
            }
        }
    }
}

/// A plane through the border of a patch (every wall of a domed block
/// meets its roof so) meets it along that border: the surface's own
/// iso-curve, found at once. Until 2026-10-13 the marcher split every
/// cell along the border down to the floor, took half a minute at scale
/// 1e-3, and found nothing, because no cell edge shows a change of sign
/// along a zero set that lies on the rectangle's edge; and a B-spline
/// chart had only sampled derivative bounds, which rarely let a cell be
/// called simple.
#[test]
fn a_plane_through_the_border_of_a_patch_meets_it_along_the_border() {
    for k in SCALES {
        for nurbs in [false, true] {
            let roof = craft3d_testkit::trimmed::domed_block(
                &Frame::WORLD,
                4.0 * k,
                3.0 * k,
                2.0 * k,
                nurbs,
            )
            .unwrap()
            .body
            .faces
            .iter()
            .map(|f| f.surface.clone())
            .find(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)))
            .unwrap();
            let t = tol(k);
            let within = Bbox3::from_points([
                Vec3::new(-1.0, -1.0, -1.0) * k,
                Vec3::new(5.0, 4.0, 4.0) * k,
            ]);
            // The wall x = 0 holds the roof's border u = 0; the wall y = 3
            // its border v = 1.
            for (origin, normal, from, to) in [
                (
                    Vec3::ZERO,
                    Vec3::X,
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.0, 3.0, 0.0),
                ),
                (
                    Vec3::new(0.0, 3.0, 0.0),
                    Vec3::Y,
                    Vec3::new(0.0, 3.0, 0.0),
                    Vec3::new(4.0, 3.0, 0.0),
                ),
            ] {
                let wall = plane(Frame::new(origin * k, normal, Vec3::Z).unwrap());
                let clock = WorkClock::start();
                let m = meeting(meet_surfaces(&roof, &wall, &within, &t).unwrap());
                let took = clock.elapsed();
                assert_eq!(
                    m.curves.len(),
                    1,
                    "scale {k} nurbs {nurbs}: {:?}",
                    m.curves.len()
                );
                assert!(m.points.is_empty());
                check_meeting(&m, &roof, &wall, &within, &t);
                let curve = &m.curves[0];
                // From one corner of the roof to the next, along the wall.
                let ends = [start(curve), end(curve)];
                let (from, to) = (from * k, to * k);
                let at = |p: Vec3, q: Vec3| {
                    (p.x - q.x).abs() <= t.linear && (p.y - q.y).abs() <= t.linear
                };
                assert!(
                    (at(ends[0], from) && at(ends[1], to))
                        || (at(ends[0], to) && at(ends[1], from)),
                    "scale {k} nurbs {nurbs}: ends {ends:?}"
                );
                assert!(curve.deviation <= t.linear);
                assert!(
                    took.as_secs() < 10,
                    "scale {k} nurbs {nurbs}: took {took:?}"
                );
            }
        }
    }
}

/// A bicubic patch that is the graph over the unit square of the frame's
/// xy plane, times `k`, of
/// `lift + a (x − cx)² + b (y − cy)² + c (x − cx)(y − cy)`:
/// a height field with one critical point, at `(cx, cy)`, whose Hessian is
/// known. As a Bézier patch or as the same patch as a NURBS surface.
fn quadric_patch(
    frame: &Frame,
    k: f64,
    (cx, cy): (f64, f64),
    (a, b, c): (f64, f64, f64),
    lift: f64,
    nurbs: bool,
) -> Surface {
    let mut coefficient = [[0.0_f64; 4]; 4];
    coefficient[0][0] = lift + a * cx * cx + b * cy * cy + c * cx * cy;
    coefficient[1][0] = -2.0 * a * cx - c * cy;
    coefficient[0][1] = -2.0 * b * cy - c * cx;
    coefficient[2][0] = a;
    coefficient[0][2] = b;
    coefficient[1][1] = c;
    height_patch(frame, k, &coefficient, nurbs)
}

/// A bicubic patch that is the graph over the unit square of the frame's
/// xy plane, times `k`, of the polynomial whose coefficient of `x^p y^q`
/// is `coefficient[p][q]`.
fn height_patch(frame: &Frame, k: f64, coefficient: &[[f64; 4]; 4], nurbs: bool) -> Surface {
    // The Bézier ordinate of x^p at control point i of a cubic: the
    // blossom of the monomial at i ones and 3 − i zeros.
    let blossom = |p: usize, i: usize| -> f64 {
        let i = i as f64;
        match p {
            0 => 1.0,
            1 => i / 3.0,
            2 => i * (i - 1.0) / 6.0,
            _ => i * (i - 1.0) * (i - 2.0) / 6.0,
        }
    };
    let mut net = [[Vec3::ZERO; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, point) in row.iter_mut().enumerate() {
            let mut z = 0.0;
            for (p, of_x) in coefficient.iter().enumerate() {
                for (q, value) in of_x.iter().enumerate() {
                    z += value * blossom(p, i) * blossom(q, j);
                }
            }
            *point = frame.point(k * i as f64 / 3.0, k * j as f64 / 3.0, k * z);
        }
    }
    let patch = craft3d_geom::BezierPatch::bicubic(net).unwrap();
    if nurbs {
        Surface::Nurbs(craft3d_geom::NurbsSurface::from_bezier(&patch).unwrap())
    } else {
        Surface::Bezier(patch)
    }
}

/// The roof of a domed block.
fn roof(frame: &Frame, size: (f64, f64, f64), nurbs: bool) -> Surface {
    craft3d_testkit::trimmed::domed_block(frame, size.0, size.1, size.2, nurbs)
        .unwrap()
        .body
        .faces
        .iter()
        .map(|f| f.surface.clone())
        .find(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)))
        .unwrap()
}

/// Two roofs set against each other so that they touch at one point and
/// cross elsewhere (tangent/dome-dome/005 of the boolean corpus). Around
/// the touch the gradient of the distance is small over a wide area (the
/// weaker eigenvalue of its Hessian is a seventeenth of the stronger), and
/// a cell is called simple only where the gradient exceeds the bound on
/// the second derivatives times the cell. Until 2026-10-13 that bound was
/// four times the largest curvature anywhere on the other roof, the cells
/// around the point were split down to the floor at every level, and the
/// pair was refused after 400,000 cells and seven seconds as "tangent or
/// nearly coincident over too large an area". Now the point is looked
/// for as soon as cells pile up, the disc around it is settled by its
/// second-order model, and the bound is taken from the curvature nearby.
#[test]
fn roofs_that_touch_at_a_point_and_cross_elsewhere_meet_in_both() {
    for nurbs in [false, true] {
        let lower = roof(
            &Frame::new(Vec3::new(-0.625, -1.25, 0.125), Vec3::Z, Vec3::X).unwrap(),
            (1.75, 1.0, 2.125),
            nurbs,
        );
        let upper = roof(
            &Frame::new(
                Vec3::new(-0.9687694778582878, 0.14324144951977846, 3.056784691023382),
                Vec3::new(0.0959496952727907, -0.0796694558935784, -0.9921927402348193),
                Vec3::new(
                    0.995371366868992,
                    0.0022407034470068995,
                    0.09607716307956281,
                ),
            )
            .unwrap(),
            (2.25, 1.625, 1.125),
            nurbs,
        );
        let t = tol(1.0);
        let within = cube(Vec3::new(0.0, -0.75, 2.2), 3.0);
        let clock = WorkClock::start();
        let m = meeting(meet_surfaces(&lower, &upper, &within, &t).unwrap());
        let took = clock.elapsed();
        check_meeting(&m, &lower, &upper, &within, &t);
        assert_eq!(m.points.len(), 1, "nurbs {nurbs}: {:?}", m.points);
        let touch = m.points[0];
        assert!(distance(&lower, touch) <= t.linear && distance(&upper, touch) <= t.linear);
        let (na, nb) = (
            normal_at(&lower, touch).unwrap(),
            normal_at(&upper, touch).unwrap(),
        );
        assert!(na.cross(nb).length() < 1e-5, "nurbs {nurbs}: {na:?} {nb:?}");
        // The curves keep clear of the touch: it is a point of its own.
        assert!(!m.curves.is_empty(), "nurbs {nurbs}");
        for curve in &m.curves {
            for p in samples(curve, 200) {
                assert!(p.distance(touch) > 0.01, "nurbs {nurbs}: {p:?}");
            }
        }
        assert!(took.as_secs() < 20, "nurbs {nurbs}: took {took:?}");
    }
}

/// Two freeform patches that touch at one point, a bowl on a cap: the
/// meeting is that point and nothing else. (The difference of the two
/// height fields has a minimum there whose smaller eigenvalue is a tenth
/// of the larger.)
#[test]
fn freeform_patches_that_touch_at_a_point_meet_in_that_point() {
    for k in SCALES {
        for nurbs in [false, true] {
            let frame = tilted(k);
            let at = (0.4, 0.55);
            let bowl = quadric_patch(&frame, k, at, (0.9, 0.35, 0.2), 0.0, nurbs);
            let cap = quadric_patch(&frame, k, at, (-0.6, 0.2, -0.1), 0.0, nurbs);
            let t = tol(k);
            let touch = frame.point(k * at.0, k * at.1, 0.0);
            let within = cube(touch, 2.0 * k);
            let clock = WorkClock::start();
            let m = meeting(meet_surfaces(&bowl, &cap, &within, &t).unwrap());
            let took = clock.elapsed();
            assert!(m.curves.is_empty(), "scale {k} nurbs {nurbs}: {m:?}");
            assert_eq!(m.points.len(), 1, "scale {k} nurbs {nurbs}: {m:?}");
            assert!(
                m.points[0].distance(touch) <= t.linear,
                "scale {k} nurbs {nurbs}: {:?} against {touch:?}",
                m.points[0]
            );
            assert!(
                took.as_secs() < 10,
                "scale {k} nurbs {nurbs}: took {took:?}"
            );
        }
    }
}

/// Two freeform patches that are tangent at a point and cross there: the
/// difference of the height fields is a saddle, `0.5 x² − 0.32 y²` about
/// the point, so the meeting is two curves through it, along
/// `y = ± 1.25 x` seen from above, each arm ending at the point.
#[test]
fn freeform_patches_that_cross_through_a_touch_meet_in_four_arms() {
    for k in SCALES {
        for nurbs in [false, true] {
            let frame = tilted(k);
            let at = (0.45, 0.5);
            let upper = quadric_patch(&frame, k, at, (0.8, -0.12, 0.0), 0.0, nurbs);
            let lower = quadric_patch(&frame, k, at, (0.3, 0.2, 0.0), 0.0, nurbs);
            let t = tol(k);
            let touch = frame.point(k * at.0, k * at.1, 0.0);
            let within = cube(touch, 2.0 * k);
            let clock = WorkClock::start();
            let m = meeting(meet_surfaces(&upper, &lower, &within, &t).unwrap());
            let took = clock.elapsed();
            check_meeting(&m, &upper, &lower, &within, &t);
            assert!(m.points.is_empty(), "scale {k} nurbs {nurbs}: {m:?}");
            assert_eq!(
                m.curves.len(),
                4,
                "scale {k} nurbs {nurbs}: {} curves",
                m.curves.len()
            );
            let mut quadrants = [false; 4];
            for curve in &m.curves {
                let ends = [start(curve), end(curve)];
                let inner = ends
                    .iter()
                    .filter(|p| p.distance(touch) <= t.linear)
                    .count();
                assert_eq!(inner, 1, "scale {k} nurbs {nurbs}: ends {ends:?}");
                for p in samples(curve, 40) {
                    let (x, y, _) = frame.local(p);
                    let (x, y) = (x / k - at.0, y / k - at.1);
                    // On y = ± 1.25 x: the patches are exact quadrics.
                    assert!(
                        (y.abs() - 1.25 * x.abs()).abs() <= 4.0 * t.linear / k,
                        "scale {k} nurbs {nurbs}: ({x}, {y})"
                    );
                    if x.phypot(y) > 0.01 {
                        quadrants[usize::from(x > 0.0) + 2 * usize::from(y > 0.0)] = true;
                    }
                }
                // Out to the border of the patches.
                let outer = ends.iter().map(|p| p.distance(touch)).fold(0.0, f64::max);
                assert!(outer > 0.4 * k, "scale {k} nurbs {nurbs}: ends {ends:?}");
            }
            assert_eq!(quadrants, [true; 4], "scale {k} nurbs {nurbs}");
            assert!(
                took.as_secs() < 10,
                "scale {k} nurbs {nurbs}: took {took:?}"
            );
        }
    }
}

/// The bowl held ten tolerances above the cap: they do not meet.
#[test]
fn freeform_patches_that_nearly_touch_do_not_meet() {
    for k in SCALES {
        for nurbs in [false, true] {
            let frame = tilted(k);
            let at = (0.4, 0.55);
            let bowl = quadric_patch(&frame, k, at, (0.9, 0.35, 0.2), 1e-5, nurbs);
            let cap = quadric_patch(&frame, k, at, (-0.6, 0.2, -0.1), 0.0, nurbs);
            let t = tol(k);
            let within = cube(frame.point(k * at.0, k * at.1, 0.0), 2.0 * k);
            let clock = WorkClock::start();
            let met = meet_surfaces(&bowl, &cap, &within, &t).unwrap();
            let took = clock.elapsed();
            assert!(
                matches!(met, craft3d_geom::intersect::SurfaceMeeting::None),
                "scale {k} nurbs {nurbs}: {met:?}"
            );
            assert!(
                took.as_secs() < 10,
                "scale {k} nurbs {nurbs}: took {took:?}"
            );
        }
    }
}

/// The same crossing through a touch with the surfaces a tenth of a
/// tolerance apart at the point: within half the tolerance, so it is read
/// as the crossing, and the four arms still end at the point, each
/// leaving it along its own direction of the crossing. (`f = 0` itself is
/// two branches there that turn away half a thousandth before the point.
/// Until 2026-10-13 the arms were fitted to them on the way in, so two
/// arms reached the point from the same side, and a boolean sorted the
/// four pieces around the vertex wrongly: two lumps that touch came out
/// as one shell, near/dome-dome/012 of the boolean corpus.)
#[test]
fn a_crossing_a_tenth_of_a_tolerance_apart_is_drawn_to_the_point() {
    for k in SCALES {
        for nurbs in [false, true] {
            let frame = tilted(k);
            let at = (0.45, 0.5);
            let upper = quadric_patch(&frame, k, at, (0.8, -0.12, 0.0), 1e-7, nurbs);
            let lower = quadric_patch(&frame, k, at, (0.3, 0.2, 0.0), 0.0, nurbs);
            let t = tol(k);
            let touch = frame.point(k * at.0, k * at.1, 0.0);
            let within = cube(touch, 2.0 * k);
            let m = meeting(meet_surfaces(&upper, &lower, &within, &t).unwrap());
            check_meeting(&m, &upper, &lower, &within, &t);
            assert_eq!(m.curves.len(), 4, "scale {k} nurbs {nurbs}");
            let mut quadrants = [false; 4];
            for curve in &m.curves {
                let (first, last) = curve.range;
                let (node, inwards) = if start(curve).distance(touch) <= t.linear {
                    (first, 1.0)
                } else {
                    assert!(
                        end(curve).distance(touch) <= t.linear,
                        "scale {k} nurbs {nurbs}: an arm that does not end at the point"
                    );
                    (last, -1.0)
                };
                // Two ten-thousandths out (the branches of `f = 0` turn
                // away at 5.6 of them, on the y axis, 39° from here): along
                // y = ± 1.25 x within 15° (the arm starts on a branch four
                // times as far out, a few degrees off the direction).
                let p = curve.curve.eval(node + inwards * 2e-4 * k);
                let (x, y, _) = frame.local(p);
                let (x, y) = (x / k - at.0, y / k - at.1);
                let off = (y.abs().patan2(x.abs()) - 1.25_f64.patan()).abs();
                assert!(
                    off < 15.0_f64.to_radians(),
                    "scale {k} nurbs {nurbs}: an arm leaves towards ({x}, {y})"
                );
                quadrants[usize::from(x > 0.0) + 2 * usize::from(y > 0.0)] = true;
            }
            assert_eq!(quadrants, [true; 4], "scale {k} nurbs {nurbs}");
        }
    }
}

/// The same, wherever the crossing lies on the patches and whichever way
/// they are a little apart (a tenth or three tenths of a tolerance): four
/// arms, each with one end on the point, and nothing drawn twice. (A branch marched in from the border ends on
/// the point when it comes within the radius at which the arms are
/// seeded. Until 2026-10-14 it had to come within one step of it, and a
/// step that landed nearer than the place where `f = 0` turns away went
/// past the point and on along the next arm to the border: that arm was
/// then there twice, and a boolean refused it as "two curves leave a
/// point in the same direction"; near/block-spline/012 and three more
/// cases of the boolean corpus. Here it was round 14 at scale 1: three
/// curves, one of them through the place without an end on it.)
#[test]
fn a_crossing_a_little_apart_is_four_arms_wherever_it_lies() {
    const ROUNDS: usize = 24;
    let mut rng = Rng(20_261_014);
    let mut wrong = Vec::new();
    for k in SCALES {
        for round in 0..ROUNDS {
            let nurbs = round % 2 == 1;
            let frame = tilted(k);
            let at = (rng.range(0.25, 0.75), rng.range(0.25, 0.75));
            let apart = [1e-7, -1e-7, 3e-7, -3e-7][round % 4];
            let upper = quadric_patch(&frame, k, at, (0.8, -0.12, 0.0), apart, nurbs);
            let lower = quadric_patch(&frame, k, at, (0.3, 0.2, 0.0), 0.0, nurbs);
            let t = tol(k);
            let touch = frame.point(k * at.0, k * at.1, 0.0);
            let within = cube(touch, 2.0 * k);
            let m = meeting(meet_surfaces(&upper, &lower, &within, &t).unwrap());
            let what = format!("scale {k}, round {round}, at {at:?}, {apart} apart");
            if m.curves.len() != 4 {
                wrong.push(format!("{what}: {} curves", m.curves.len()));
            }
            for (i, curve) in m.curves.iter().enumerate() {
                let ends = [start(curve), end(curve)];
                let inner = ends
                    .iter()
                    .filter(|p| p.distance(touch) <= t.linear)
                    .count();
                if inner != 1 {
                    wrong.push(format!("{what}: curve {i} has {inner} ends on the point"));
                }
                // Away from the point no other curve is anywhere near.
                for (j, other) in m.curves.iter().enumerate().skip(i + 1) {
                    let drawn = Lines::of(
                        &Meeting {
                            curves: vec![other.clone()],
                            points: Vec::new(),
                        },
                        200,
                    );
                    let together = samples(curve, 24)
                        .iter()
                        .any(|p| p.distance(touch) > 0.02 * k && drawn.distance(*p) < 1e-3 * k);
                    if together {
                        wrong.push(format!("{what}: curves {i} and {j} run together"));
                    }
                }
            }
            // (Held to the surfaces in one round per scale: that takes
            // longer than everything else here. Round 14 is the one that
            // went wrong at scale 1.)
            if round == 14 {
                check_meeting(&m, &upper, &lower, &within, &t);
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A cylinder that fits the hole of a torus touches it along the inner
/// equator, and one that fits around it along the outer one: the surfaces
/// are tangent all along a circle and keep to one side of each other, so
/// they meet in points of that circle and in no curve. (The Hessian of
/// the distance is degenerate there, which says neither touch nor
/// crossing; until 2026-10-13 that was refused as "surfaces that touch
/// along a curve, or with more than second-order contact". Now a contact
/// around which the distance keeps one sign counts as the touch it is.)
/// A cylinder a little wider than the hole cuts the torus in two circles.
#[test]
fn a_cylinder_that_fits_a_torus_touches_it_along_a_circle() {
    for k in SCALES {
        let frame = tilted(k);
        let ring = torus(frame, 2.0 * k, 0.5 * k);
        let t = tol(k);
        let within = cube(frame.origin(), 4.0 * k);
        for radius in [1.5, 2.5] {
            let tube = cylinder(frame, radius * k);
            for (chart, field) in [(&ring, &tube), (&tube, &ring)] {
                let m = meeting(meet_surfaces_numeric(chart, field, &within, &t).unwrap());
                assert!(m.curves.is_empty(), "scale {k} radius {radius}: {m:?}");
                assert!(!m.points.is_empty(), "scale {k} radius {radius}");
                for p in &m.points {
                    let (x, y, z) = frame.local(*p);
                    assert!(
                        (x.phypot(y) - radius * k).abs() <= t.linear && z.abs() <= 64.0 * t.linear,
                        "scale {k} radius {radius}: a touch at ({x}, {y}, {z})"
                    );
                }
            }
        }
        let wider = cylinder(frame, 1.6 * k);
        let m = meeting(meet_surfaces_numeric(&ring, &wider, &within, &t).unwrap());
        check_meeting(&m, &ring, &wider, &within, &t);
        assert_eq!(m.curves.len(), 2, "scale {k}");
        assert!(m.points.is_empty(), "scale {k}: {:?}", m.points);
    }
}

/// A contact the Hessian does not classify and that is a crossing all the
/// same: the patch `0.8 (x − 0.45)² − 0.6 (y − 0.5)³` against the plane it
/// is tangent to. The two meet in a cusp, `x − 0.45 = ± sqrt(0.75 (y −
/// 0.5)³)`, whose two arms leave the point in the same direction; between
/// them the patch is under the plane, in a wedge that narrows to nothing
/// at the point. Rings of samples around the point miss that wedge once
/// they are small, and the contact then passes for one-sided. That must
/// not lose the cusp: a disc around a touch only keeps cells from being
/// split, and the arms, found outside it, are marched into it. The answer
/// is the cusp drawn to where its wedge is narrower than the tolerance
/// (64 tolerances from the point at most; 19 measured), or a refusal,
/// never a touch alone and never arms that end short of it.
#[test]
fn a_degenerate_contact_that_crosses_in_a_cusp_is_not_a_touch() {
    for k in SCALES {
        for nurbs in [false, true] {
            let frame = tilted(k);
            let (cx, cy) = (0.45, 0.5);
            let mut coefficient = [[0.0_f64; 4]; 4];
            coefficient[2][0] = 0.8;
            coefficient[1][0] = -1.6 * cx;
            coefficient[0][0] = 0.8 * cx * cx + 0.6 * cy * cy * cy;
            coefficient[0][1] = -1.8 * cy * cy;
            coefficient[0][2] = 1.8 * cy;
            coefficient[0][3] = -0.6;
            let patch = height_patch(&frame, k, &coefficient, nurbs);
            let flat = plane(frame);
            let t = tol(k);
            let cusp = frame.point(k * cx, k * cy, 0.0);
            let within = cube(cusp, 4.0 * k);
            for (chart, field) in [(&patch, &flat), (&flat, &patch)] {
                let m = match meet_surfaces_numeric(chart, field, &within, &t) {
                    Ok(found) => meeting(found),
                    Err(
                        craft3d_geom::GeomError::Unsupported(_)
                        | craft3d_geom::GeomError::NoConvergence(_),
                    ) => continue,
                    Err(other) => panic!("scale {k} nurbs {nurbs}: {other:?}"),
                };
                check_meeting(&m, chart, field, &within, &t);
                assert!(!m.curves.is_empty(), "scale {k} nurbs {nurbs}: {m:?}");
                let gap = Lines::of(&m, 400).distance(cusp);
                assert!(
                    gap <= 64.0 * t.linear,
                    "scale {k} nurbs {nurbs}: the arms end {gap} short of the cusp"
                );
            }
        }
    }
}

/// A cylinder inside the tube of a torus, its axis square to the torus'
/// axis, touching the top of the tube from inside: the contact is of the
/// fourth order. Across the cylinder the two are tangent circles; along
/// it the tube bends down and away, so that on either side of the point
/// the cylinder pokes out of the tube in a sliver between two curves that
/// leave the point in the same direction (a tacnode: in the distance
/// `f ≈ −s² + (2/3) s y² + y⁴/9` near the point, with `s` across and `y`
/// along the cylinder in units of the minor radius, the curves are
/// `s ≈ 0.81 y²` and `s ≈ −0.14 y²`). The meeting is two loops, one on
/// either side, each leaving the point along one of its curves and coming
/// back along the other: four ends on the point and none elsewhere.
/// (Until 2026-10-15 the stationary search took no step where the Hessian
/// is flat, no point was made there, and the branches were marched
/// through it until the march stalled: the random runs' seed 2001169.)
///
/// `lift` moves the cylinder up (out of the tube) by that share of the
/// tolerance; `radius` is the cylinder's radius over the torus' minor
/// radius.
fn tacnode_case(k: f64, radius: f64, lift: f64) -> (Surface, Surface, Vec3, Frame) {
    let frame = tilted(k);
    let (major, minor) = (1.5 * k, 0.5 * k);
    let rho = radius * minor;
    let ring = torus(frame, major, minor);
    let t = tol(k);
    let axis = Frame::new(
        frame.point(major, 0.0, minor - rho + lift * t.linear),
        frame.y(),
        frame.x(),
    )
    .unwrap();
    let tube = cylinder(axis, rho);
    (ring, tube, frame.point(major, 0.0, minor), frame)
}

/// Points of the exact meeting of [`tacnode_case`] (with no lift): for an
/// angle `φ` round the cylinder from its top, the point of the cylinder
/// there at `y = ± sqrt((R + w)² − x²)`, `w = sqrt(r² − z²)`.
fn tacnode_reference(k: f64, radius: f64, frame: &Frame) -> Vec<Vec3> {
    let (major, minor) = (1.5 * k, 0.5 * k);
    let rho = radius * minor;
    let mut out = Vec::new();
    for i in 1..400 {
        let phi = std::f64::consts::TAU * f64::from(i) / 400.0;
        let x = major + rho * phi.psin();
        let z = minor - rho + rho * phi.pcos();
        let w = (minor * minor - z * z).max(0.0).sqrt();
        let y = ((major + w) * (major + w) - x * x).max(0.0).sqrt();
        for side in [1.0, -1.0] {
            out.push(frame.point(x, side * y, z));
        }
    }
    out
}

#[test]
fn a_cylinder_touching_the_top_of_a_torus_tube_from_inside_meets_it_in_a_tacnode() {
    for k in SCALES {
        for (radius, lift) in [
            (0.5, 0.0),
            (0.3, 0.0),
            (0.2, 0.0),
            (0.1, 0.0),
            (0.5, 0.3),
            (0.5, -0.3),
        ] {
            let (ring, tube, touch, frame) = tacnode_case(k, radius, lift);
            let t = tol(k);
            let within = cube(frame.origin(), 2.5 * k);
            let what = format!("scale {k}, radius {radius}, lift {lift}");
            let m = meeting(
                meet_surfaces(&ring, &tube, &within, &t).unwrap_or_else(|e| panic!("{what}: {e}")),
            );
            check_meeting(&m, &ring, &tube, &within, &t);
            assert!(m.points.is_empty(), "{what}: {:?}", m.points);
            // Every curve begins and ends on one point of contact. Where
            // it lies along the arms is ill-conditioned (the surfaces are
            // within `y⁴` of each other along it): it is on both surfaces
            // within the tolerance and near the true point.
            let vertex = start(&m.curves[0]);
            assert!(
                distance(&ring, vertex) <= t.linear && distance(&tube, vertex) <= t.linear,
                "{what}: a vertex off the surfaces"
            );
            assert!(
                vertex.distance(touch) <= 64.0 * t.linear,
                "{what}: the vertex is {} from the point of contact",
                vertex.distance(touch)
            );
            let ends: usize = m
                .curves
                .iter()
                .map(|c| {
                    usize::from(start(c).distance(vertex) <= t.linear)
                        + usize::from(end(c).distance(vertex) <= t.linear)
                })
                .sum();
            assert_eq!(
                ends,
                2 * m.curves.len(),
                "{what}: {} curves",
                m.curves.len()
            );
            assert_eq!(ends, 4, "{what}");
            // Along the exact meeting and only there.
            if lift == 0.0 {
                let drawn = Lines::of(&m, 20_000);
                for p in tacnode_reference(k, radius, &frame) {
                    let off = drawn.distance(p);
                    assert!(
                        off <= 4.0 * t.linear,
                        "{what}: {off} from the exact meeting"
                    );
                }
            }
        }
    }
}
