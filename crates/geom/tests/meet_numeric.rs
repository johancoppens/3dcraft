//! The numeric marcher: against the closed forms it can be forced onto,
//! and on the classic curves that have no closed form here.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_geom::intersect::{Meeting, meet_surfaces, meet_surfaces_numeric};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance};

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
