//! The marcher on curves with a known shape (crossing cylinders, Viviani's
//! curve, Villarceau circles), on freeform patches against root finding
//! along their iso-lines, and on touching and degenerate configurations.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_geom::intersect::{Meeting, MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{BezierPatch, GeomError, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Transform, Vec3};
use std::f64::consts::TAU;

/// The length of a meeting curve (a polyline of 2000 chords).
fn length(curve: &MeetingCurve) -> f64 {
    samples(curve, 2000)
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .sum()
}

/// `meet_surfaces`, asked twice and with the surfaces swapped: the same
/// answer, bit for bit.
fn solve(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    t: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let found = meet_surfaces(a, b, within, t);
    assert_eq!(found, meet_surfaces(a, b, within, t), "not deterministic");
    assert_eq!(
        found,
        meet_surfaces(b, a, within, t),
        "depends on the order"
    );
    if let Ok(SurfaceMeeting::Meets(m)) = &found {
        check_meeting(m, a, b, within, t);
    }
    found
}

/// A meeting in a few words, for failure messages.
fn brief(m: &Meeting) -> String {
    let mut out = format!("{} points {:?};", m.points.len(), m.points);
    for c in &m.curves {
        out += &format!(
            "\n  {} from {:?} to {:?}, length {}",
            if c.closed { "closed" } else { "open" },
            start(c),
            end(c),
            length(c)
        );
    }
    out
}

/// Every end of every curve is at one of `places`.
fn ends_at(m: &Meeting, places: &[Vec3], slack: f64) {
    for curve in &m.curves {
        for p in [start(curve), end(curve)] {
            assert!(
                places.iter().any(|q| q.distance(p) <= slack),
                "a curve ends at {p:?}, not at a crossing point"
            );
        }
    }
}

#[test]
fn unequal_crossing_cylinders_meet_in_two_closed_quartics() {
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let within = cube(f.origin(), 12.0 * s);
        let big = cylinder(f, 2.0 * s);
        let small = cylinder(
            Frame::new(f.point(0.0, 0.3 * s, 0.0), f.x(), f.z()).unwrap(),
            s,
        );
        let m = meeting(solve(&big, &small, &within, &t).unwrap());
        assert_eq!(m.curves.len(), 2, "{}", brief(&m));
        assert!(m.curves.iter().all(|c| c.closed && !c.exact && !c.tangent));
        // x = ±√(4 - (0.3 + cos t)²), y = 0.3 + cos t, z = sin t.
        let lines = Lines::of(&m, 4000);
        let mut expected = 0.0;
        let mut previous: Option<Vec3> = None;
        for k in 0..=2000 {
            let angle = TAU * f64::from(k) / 2000.0;
            let y = 0.3 + angle.cos();
            let x = (4.0 - y * y).sqrt();
            for side in [1.0, -1.0] {
                let p = f.point(side * x * s, y * s, angle.sin() * s);
                assert!(
                    lines.distance(p) <= 1e-5 * s,
                    "the quartic is missing at {angle}"
                );
            }
            let p = f.point(x * s, y * s, angle.sin() * s);
            expected += previous.map_or(0.0, |q| q.distance(p));
            previous = Some(p);
        }
        for curve in &m.curves {
            assert!((length(curve) - expected).abs() <= 1e-4 * s);
        }
    }
}

#[test]
fn a_sphere_tangent_inside_a_cylinder_gives_vivianis_curve() {
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let within = cube(f.origin(), 12.0 * s);
        let ball = sphere(f, 2.0 * s);
        let tube = cylinder(f.at(f.point(s, 0.0, 0.0)).unwrap(), s);
        let m = meeting(solve(&ball, &tube, &within, &t).unwrap());
        // A figure of eight: two lobes that start and end on its double
        // point, where the surfaces are tangent.
        let node = f.point(2.0 * s, 0.0, 0.0);
        assert_eq!(m.curves.len(), 2, "{}", brief(&m));
        ends_at(&m, &[node], 2.0 * t.linear);
        assert!(m.points.is_empty());
        assert!(m.curves.iter().all(|c| !c.closed && !c.tangent));
        let lines = Lines::of(&m, 4000);
        let mut expected = 0.0;
        let mut previous: Option<Vec3> = None;
        for k in 0..=4000 {
            let angle = 2.0 * TAU * f64::from(k) / 4000.0;
            let p = f.point(
                s * (1.0 + angle.cos()),
                s * angle.sin(),
                2.0 * s * (0.5 * angle).sin(),
            );
            assert!(lines.distance(p) <= 1e-5 * s, "missing at {angle}");
            expected += previous.map_or(0.0, |q| q.distance(p));
            previous = Some(p);
        }
        let total: f64 = m.curves.iter().map(length).sum();
        assert!(
            (total - expected).abs() <= 1e-4 * s,
            "{total} vs {expected}"
        );
        // One lobe above, one below.
        let above = |c: &MeetingCurve| f.local(samples(c, 2)[1]).2 > 0.0;
        assert_ne!(above(&m.curves[0]), above(&m.curves[1]));
    }
}

#[test]
fn a_bitangent_plane_cuts_a_torus_in_villarceau_circles() {
    for s in SCALES {
        let f = tilted(s);
        let t = tol(s);
        let within = cube(f.origin(), 12.0 * s);
        let (major, minor) = (4.0 * s, s);
        let ring = torus(f, major, minor);
        let lean = (minor / major).asin();
        let leaning = |angle: f64| {
            plane(Frame::new(f.origin(), f.vector(-angle.sin(), 0.0, angle.cos()), f.y()).unwrap())
        };
        let m = meeting(solve(&leaning(lean), &ring, &within, &t).unwrap());
        // Two circles of the major radius about (0, ±r, 0), crossing at
        // the two points where the plane touches the torus: four arcs.
        let reach = (major * major - minor * minor).sqrt();
        let touch = [
            f.point(reach * lean.cos(), 0.0, reach * lean.sin()),
            f.point(-reach * lean.cos(), 0.0, -reach * lean.sin()),
        ];
        assert_eq!(m.curves.len(), 4, "{}", brief(&m));
        ends_at(&m, &touch, 2.0 * t.linear);
        assert!(m.points.is_empty());
        let centers = [f.point(0.0, minor, 0.0), f.point(0.0, -minor, 0.0)];
        for curve in &m.curves {
            let on = |c: Vec3| {
                samples(curve, 100)
                    .iter()
                    .all(|p| (p.distance(c) - major).abs() <= 2.0 * t.linear)
            };
            assert!(
                on(centers[0]) != on(centers[1]),
                "an arc lies on one circle"
            );
        }
        let total: f64 = m.curves.iter().map(length).sum();
        assert!((total - 2.0 * TAU * major).abs() <= 1e-4 * s);

        // Steeper or flatter than bitangent: two closed loops.
        for turn in [1.3, 0.7] {
            let m = meeting(solve(&leaning(lean * turn), &ring, &within, &t).unwrap());
            assert_eq!(m.curves.len(), 2, "turn {turn}: {}", brief(&m));
            assert!(m.curves.iter().all(|c| c.closed));
        }
    }
}

/// A bicubic patch with bumps.
fn bumpy(s: f64) -> BezierPatch {
    let mut net = [[Vec3::ZERO; 4]; 4];
    for (i, row) in net.iter_mut().enumerate() {
        for (j, p) in row.iter_mut().enumerate() {
            let (x, y) = (i as f64, j as f64 * 1.5);
            *p = Vec3::new(x, y, ((i * 7 + j * 3) % 5) as f64 * 0.3 - 0.2 * x) * s;
        }
    }
    BezierPatch::bicubic(net).unwrap()
}

/// A rational surface with an inner knot each way.
fn rational(s: f64) -> NurbsSurface {
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (i as f64 * 0.8, j as f64 * 1.1);
            ctrl.push(Vec3::new(x, y, ((i * 3 + j * 2) % 4) as f64 * 0.35 - 0.1 * y) * s);
            weights.push(1.0 + 0.5 * ((i + 2 * j) % 3) as f64);
        }
    }
    NurbsSurface::new(3, 3, knots.clone(), knots, 5, 5, ctrl, weights).unwrap()
}

/// Where the iso-lines of a freeform surface cross a plane, by sampling
/// each line finely and bisecting: independent of the marcher.
fn iso_roots(surface: &Surface, origin: Vec3, normal: Vec3) -> Vec<Vec3> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let mut roots = Vec::new();
    for along_u in [true, false] {
        for line in 0..40 {
            let fixed = (f64::from(line) + 0.5) / 40.0;
            let at = |x: f64| {
                if along_u {
                    (u0 + (u1 - u0) * x, v0 + (v1 - v0) * fixed)
                } else {
                    (u0 + (u1 - u0) * fixed, v0 + (v1 - v0) * x)
                }
            };
            let value = |x: f64| (surface.eval(at(x).0, at(x).1) - origin).dot(normal);
            for k in 0..600 {
                let (mut lo, mut hi) = (f64::from(k) / 600.0, f64::from(k + 1) / 600.0);
                if (value(lo) >= 0.0) == (value(hi) >= 0.0) {
                    continue;
                }
                for _ in 0..60 {
                    let mid = 0.5 * (lo + hi);
                    if (value(mid) >= 0.0) == (value(lo) >= 0.0) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let (u, v) = at(0.5 * (lo + hi));
                roots.push(surface.eval(u, v));
            }
        }
    }
    roots
}

#[test]
fn a_plane_cuts_freeform_patches_where_their_iso_lines_cross_it() {
    for s in SCALES {
        let t = tol(s);
        let within = cube(Vec3::new(1.5, 2.0, 0.0) * s, 9.0 * s);
        let surfaces = [
            Surface::Bezier(bumpy(s)),
            Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(s)).unwrap()),
            Surface::Nurbs(rational(s)),
        ];
        for (index, surface) in surfaces.iter().enumerate() {
            for (height, lean) in [(0.35, 0.1), (0.1, -0.3), (0.6, 0.02)] {
                let origin = Vec3::new(0.0, 0.0, height * s);
                let normal = Vec3::new(lean, 0.5 * lean, 1.0).normalized().unwrap();
                let cut = plane(Frame::new(origin, normal, Vec3::X).unwrap());
                let roots = iso_roots(surface, origin, normal);
                let found = solve(&cut, surface, &within, &t).unwrap();
                if roots.is_empty() {
                    assert_eq!(found, SurfaceMeeting::None);
                    continue;
                }
                let m = meeting(found);
                assert!(m.curves.iter().all(|c| !c.exact && !c.tangent));
                let lines = Lines::of(&m, 4000);
                for p in &roots {
                    assert!(
                        lines.distance(*p) <= 1e-5 * s,
                        "surface {index}, plane at {height}: the crossing at {p:?} is on none of {} curves",
                        m.curves.len()
                    );
                }
                // And nothing but crossings: every curve is in the plane.
                for curve in &m.curves {
                    for p in samples(curve, 100) {
                        assert!((p - origin).dot(normal).abs() <= t.linear);
                    }
                }
            }
        }
        // The polynomial NURBS is the Bézier patch: the same curves.
        let origin = Vec3::new(0.0, 0.0, 0.35 * s);
        let cut = plane(Frame::new(origin, Vec3::Z, Vec3::X).unwrap());
        let a = meeting(solve(&cut, &surfaces[0], &within, &t).unwrap());
        let b = meeting(solve(&cut, &surfaces[1], &within, &t).unwrap());
        assert_eq!(a.curves.len(), b.curves.len());
        let total = |m: &Meeting| m.curves.iter().map(length).sum::<f64>();
        assert!((total(&a) - total(&b)).abs() <= 1e-4 * s);
    }
}

#[test]
fn freeform_patches_meet_each_other_and_curved_surfaces() {
    let t = tol(1.0);
    let within = cube(Vec3::new(1.5, 2.0, 0.0), 9.0);
    let patch = Surface::Bezier(bumpy(1.0));
    let shifted = rational(1.0)
        .transformed(&Transform::translation(Vec3::new(0.2, 0.1, 0.15)))
        .unwrap();
    for b in [
        sphere(Frame::WORLD.at(Vec3::new(1.5, 2.0, 0.2)).unwrap(), 1.4),
        cylinder(
            Frame::new(Vec3::new(1.5, 2.0, 0.0), Vec3::new(0.1, 0.2, 1.0), Vec3::X).unwrap(),
            0.9,
        ),
        torus(Frame::WORLD.at(Vec3::new(1.5, 2.2, 0.3)).unwrap(), 1.2, 0.4),
        cone(
            Frame::WORLD.at(Vec3::new(1.5, 2.0, -1.0)).unwrap(),
            0.3,
            0.5,
        ),
        Surface::Nurbs(shifted),
    ] {
        let m = meeting(solve(&patch, &b, &within, &t).unwrap());
        assert!(!m.curves.is_empty(), "{}", b.kind());
        // Each curve closes or ends on the edge of a patch.
        for curve in &m.curves {
            if curve.closed {
                continue;
            }
            for p in [start(curve), end(curve)] {
                let on_edge = |surface: &Surface| {
                    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                        return false;
                    }
                    let ((u0, u1), (v0, v1)) = surface.domain();
                    let (u, v) = surface.project(p).unwrap();
                    let near = |x: f64, a: f64, b: f64| {
                        (x - a).abs() <= 1e-5 * (b - a) || (x - b).abs() <= 1e-5 * (b - a)
                    };
                    near(u, u0, u1) || near(v, v0, v1)
                };
                assert!(
                    on_edge(&patch) || on_edge(&b),
                    "{}: a curve ends in mid-air at {p:?}",
                    b.kind()
                );
            }
        }
    }
}

#[test]
fn touching_and_degenerate_configurations_do_not_hang() {
    let f = tilted(1.0);
    let t = tol(1.0);
    let within = cube(f.origin(), 12.0);
    // On the work of this thread, which a busy machine does not stretch.
    let clock = craft3d_testkit::cost::WorkClock::start();
    // A sphere resting on a torus, off its axis: one point.
    let ring = torus(f, 4.0, 1.0);
    let ball = sphere(f.at(f.point(4.0, 0.0, 2.0)).unwrap(), 1.0);
    let m = meeting(solve(&ball, &ring, &within, &t).unwrap());
    assert!(m.curves.is_empty(), "{}", brief(&m));
    assert_eq!(m.points.len(), 1);
    assert!(m.points[0].distance(f.point(4.0, 0.0, 1.0)) <= 2.0 * t.linear);
    // Lifted off by more than the tolerance: nothing. Pushed in: a loop.
    let lifted = sphere(f.at(f.point(4.0, 0.0, 2.001)).unwrap(), 1.0);
    assert_eq!(
        solve(&lifted, &ring, &within, &t).unwrap(),
        SurfaceMeeting::None
    );
    let pushed = sphere(f.at(f.point(4.0, 0.0, 1.999)).unwrap(), 1.0);
    let m = meeting(solve(&pushed, &ring, &within, &t).unwrap());
    assert!(m.curves.len() == 1 && m.curves[0].closed, "{}", brief(&m));
    assert!(length(&m.curves[0]) < 0.5);

    // A plane tangent to a patch along a line (z = x²): the patch keeps
    // to one side of it, so they touch, in points of that line, and meet
    // in no curve. (Refused until 2026-10-13, when a contact the Hessian
    // does not classify began to be tested for keeping one side.)
    let trough = Surface::Bezier(
        BezierPatch::new(
            2,
            1,
            vec![
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(-1.0, 2.0, 1.0),
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.0, 2.0, -1.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(1.0, 2.0, 1.0),
            ],
        )
        .unwrap(),
    );
    let floor = plane(Frame::WORLD);
    let small = cube(Vec3::ZERO, 5.0);
    let m = meeting(solve(&floor, &trough, &small, &t).unwrap());
    assert!(m.curves.is_empty(), "{}", brief(&m));
    assert!(!m.points.is_empty());
    for p in &m.points {
        assert!(
            p.x.abs() <= 64.0 * t.linear && p.z.abs() <= t.linear && (0.0..=2.0).contains(&p.y),
            "a touch at {p:?}"
        );
    }
    // Lifted a little, the plane cuts it in two lines.
    let above = plane(Frame::WORLD.at(Vec3::new(0.0, 0.0, 0.01)).unwrap());
    let m = meeting(solve(&above, &trough, &small, &t).unwrap());
    assert_eq!(m.curves.len(), 2);

    // The same surface twice, and a flat patch in a plane: coincident.
    let patch = Surface::Bezier(bumpy(1.0));
    assert_eq!(
        solve(&patch, &patch.clone(), &small, &t).unwrap(),
        SurfaceMeeting::Coincident { same_normal: true }
    );
    let flat = Surface::Bezier(
        BezierPatch::new(
            1,
            1,
            vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
        )
        .unwrap(),
    );
    assert_eq!(
        solve(&floor, &flat, &small, &t).unwrap(),
        SurfaceMeeting::Coincident { same_normal: true }
    );
    let flipped = Surface::Bezier(
        BezierPatch::new(
            1,
            1,
            vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)],
        )
        .unwrap(),
    );
    assert_eq!(
        solve(&floor, &flipped, &small, &t).unwrap(),
        SurfaceMeeting::Coincident { same_normal: false }
    );
    // A patch collapsed to a point, and a box with no size.
    let dot = Surface::Bezier(BezierPatch::new(1, 1, vec![Vec3::new(0.3, 0.2, 0.1); 4]).unwrap());
    assert!(!matches!(
        solve(&dot, &ball, &small, &t),
        Ok(SurfaceMeeting::Meets(_))
    ));
    assert!(!matches!(
        solve(&dot, &patch, &small, &t),
        Ok(SurfaceMeeting::Meets(_))
    ));
    assert!(solve(&patch, &ball, &cube(Vec3::ZERO, 0.0), &t).is_err());
    // A tolerance below what rounding allows at this size.
    let fine = Tolerance::new(1e-18, 1e-9).unwrap();
    assert!(solve(&patch, &ball, &small, &fine).is_err());
    assert!(clock.elapsed().as_secs() < 60, "took {:?}", clock.elapsed());
}
