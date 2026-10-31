//! A face's loops must bound a region of its surface in its parameters
//! (`Defect::LoopsBoundNoRegion`): one outer loop counter-clockwise about
//! the outward normal, holes clockwise inside it, no crossings. Seeded on
//! patches of a cylinder (what the wrap's rims and skins are) and a plane;
//! the reference solids at several scales stay clean.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::solids;
use craft3d_topo::{Body, BodyKind, Coedge, Defect, LoopId, check};

fn tol() -> Tolerance {
    Tolerance::default()
}

fn regionless(body: &Body) -> bool {
    check(body, &tol())
        .iter()
        .any(|d| matches!(d, Defect::LoopsBoundNoRegion { .. }))
}

/// A loop on the cylinder of radius `r` about z: angles `u0..u1`, heights
/// `z0..z1`, counter-clockwise in (u, z) unless `backwards`.
fn patch_loop(
    body: &mut Body,
    r: f64,
    (u0, u1): (f64, f64),
    (z0, z1): (f64, f64),
    backwards: bool,
) -> LoopId {
    let at = |u: f64, z: f64| Vec3::new(r * u.cos(), r * u.sin(), z);
    let corners = [at(u0, z0), at(u1, z0), at(u1, z1), at(u0, z1)];
    let v: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let arc = |z: f64| {
        let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
        Curve3::Circle(Circle::new(frame, r).unwrap())
    };
    // Bottom arc u0 -> u1, right line up, top arc (used backwards), left
    // line down.
    let bottom = body.add_edge(arc(z0), (u0, u1), v[0], v[1]);
    let right = body.add_edge(
        Curve3::Line(Line::through(corners[1], corners[2]).unwrap()),
        (0.0, z1 - z0),
        v[1],
        v[2],
    );
    let top = body.add_edge(arc(z1), (u0, u1), v[3], v[2]);
    let left = body.add_edge(
        Curve3::Line(Line::through(corners[3], corners[0]).unwrap()),
        (0.0, z1 - z0),
        v[3],
        v[0],
    );
    let mut coedges = vec![
        Coedge {
            edge: bottom,
            reversed: false,
        },
        Coedge {
            edge: right,
            reversed: false,
        },
        Coedge {
            edge: top,
            reversed: true,
        },
        Coedge {
            edge: left,
            reversed: false,
        },
    ];
    if backwards {
        coedges.reverse();
        for c in &mut coedges {
            c.reversed = !c.reversed;
        }
    }
    body.add_loop(coedges)
}

fn cylinder(r: f64) -> Surface {
    Surface::Cylinder(Cylinder::new(Frame::WORLD, r).unwrap())
}

/// A sheet of one face on the cylinder of radius `r`, with the loops
/// `make` adds.
fn sheet(r: f64, reversed: bool, make: impl Fn(&mut Body) -> Vec<LoopId>) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let loops = make(&mut body);
    let face = body.add_face(cylinder(r), reversed, loops);
    body.add_shell(vec![face]);
    body
}

#[test]
fn a_patch_of_a_cylinder_bounds_a_region_and_flipped_does_not() {
    for r in [1e-2, 1.0, 1e4] {
        let good = sheet(r, false, |b| {
            vec![patch_loop(b, r, (-0.5, 0.5), (0.0, r), false)]
        });
        assert_eq!(check(&good, &tol()), vec![], "r {r}");
        // The same loop on the face turned over: it runs clockwise about
        // the outward normal, the face would be the rest of the cylinder.
        let flipped = sheet(r, true, |b| {
            vec![patch_loop(b, r, (-0.5, 0.5), (0.0, r), false)]
        });
        assert!(regionless(&flipped), "r {r}: {:?}", check(&flipped, &tol()));
        // Turned over with its loop run the other way: a region again.
        let both = sheet(r, true, |b| {
            vec![patch_loop(b, r, (-0.5, 0.5), (0.0, r), true)]
        });
        assert_eq!(check(&both, &tol()), vec![], "r {r}");
    }
}

#[test]
fn a_patch_across_the_seam_bounds_a_region() {
    let r = 2.0;
    let good = sheet(r, false, |b| {
        vec![patch_loop(b, r, (2.8, 3.6), (0.0, 1.0), false)]
    });
    assert_eq!(check(&good, &tol()), vec![]);
}

#[test]
fn holes_must_run_clockwise_inside_the_outer_loop() {
    let r = 2.0;
    // A hole inside, clockwise: fine.
    let good = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (-1.0, 1.0), (0.0, 2.0), false),
            patch_loop(b, r, (-0.2, 0.2), (0.5, 1.0), true),
        ]
    });
    assert_eq!(check(&good, &tol()), vec![]);
    // The hole run counter-clockwise: two outer loops.
    let two = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (-1.0, 1.0), (0.0, 2.0), false),
            patch_loop(b, r, (-0.2, 0.2), (0.5, 1.0), false),
        ]
    });
    assert!(regionless(&two));
    // A hole beside the outer loop.
    let outside = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (-1.0, 1.0), (0.0, 2.0), false),
            patch_loop(b, r, (1.5, 2.0), (0.5, 1.0), true),
        ]
    });
    assert!(regionless(&outside));
    // A hole across the outer loop's border.
    let across = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (-1.0, 1.0), (0.0, 2.0), false),
            patch_loop(b, r, (0.8, 1.2), (0.5, 1.0), true),
        ]
    });
    assert!(regionless(&across));
}

#[test]
fn the_reference_solids_are_clean_at_every_scale() {
    for k in [1e-3, 1.0, 1e3] {
        let frame = Frame::new(
            Vec3::new(3.0 * k, -k, 2.0 * k),
            Vec3::new(0.3, 0.2, 1.0),
            Vec3::X,
        )
        .unwrap();
        for reference in solids::all(&frame, k).unwrap() {
            let found = check(&reference.body, &Tolerance::new(1e-6 * k, 1e-9).unwrap());
            assert_eq!(found, vec![], "k {k}");
        }
    }
}

#[test]
fn crossing_and_nested_holes_are_found() {
    let r = 2.0;
    let outer = |b: &mut Body| patch_loop(b, r, (-1.0, 1.0), (0.0, 2.0), false);
    // Two holes that overlap.
    let crossing = sheet(r, false, |b| {
        vec![
            outer(b),
            patch_loop(b, r, (-0.3, 0.1), (0.5, 1.0), true),
            patch_loop(b, r, (0.0, 0.4), (0.7, 1.2), true),
        ]
    });
    assert!(regionless(&crossing));
    // A hole inside a hole.
    let nested = sheet(r, false, |b| {
        vec![
            outer(b),
            patch_loop(b, r, (-0.5, 0.5), (0.3, 1.5), true),
            patch_loop(b, r, (-0.1, 0.1), (0.8, 1.0), true),
        ]
    });
    assert!(regionless(&nested));
}

#[test]
fn holes_across_the_seam() {
    let r = 2.0;
    // The patch and its hole both reach across u = π: fine.
    let good = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (2.0, 4.0), (0.0, 2.0), false),
            patch_loop(b, r, (3.0, 3.3), (0.5, 1.0), true),
        ]
    });
    assert_eq!(check(&good, &tol()), vec![]);
    // The hole across the seam and across the patch's border.
    let across = sheet(r, false, |b| {
        vec![
            patch_loop(b, r, (2.0, 3.2), (0.0, 2.0), false),
            patch_loop(b, r, (3.0, 3.4), (0.5, 1.0), true),
        ]
    });
    assert!(regionless(&across));
}

/// A loop on a plane that crosses itself while enclosing a positive area.
#[test]
fn a_loop_on_a_plane_that_crosses_itself_is_found() {
    let mut body = Body::new(BodyKind::Sheet);
    let points = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(4.0, 4.0, 0.0),
        Vec3::new(-1.0, 1.0, 0.0),
        Vec3::new(0.0, 3.0, 0.0),
    ];
    let v: Vec<_> = points.iter().map(|p| body.add_vertex(*p)).collect();
    let mut coedges = Vec::new();
    for i in 0..points.len() {
        let j = (i + 1) % points.len();
        let e = body.add_edge(
            Curve3::Line(Line::through(points[i], points[j]).unwrap()),
            (0.0, points[i].distance(points[j])),
            v[i],
            v[j],
        );
        coedges.push(Coedge {
            edge: e,
            reversed: false,
        });
    }
    let l = body.add_loop(coedges);
    let face = body.add_face(
        Surface::Plane(craft3d_geom::Plane::new(Frame::WORLD)),
        false,
        vec![l],
    );
    body.add_shell(vec![face]);
    assert!(regionless(&body), "{:?}", check(&body, &tol()));
}

/// The cost of the rule on a perforated tube: 100, 400 and 1600 holes in
/// rows and columns, checked whole (and a hole left outside still found at
/// 1600). Printed for docs/design/topology.md.
#[test]
fn a_perforated_tube_is_checked_in_time_about_its_holes() {
    let r = 10.0;
    for n in [10_usize, 20, 40] {
        let make = |stray: bool| {
            sheet(r, false, |b| {
                let mut loops = vec![patch_loop(b, r, (-1.5, 1.5), (0.0, 30.0), false)];
                for i in 0..n {
                    for j in 0..n {
                        let u = -1.4 + 2.8 * (i as f64 + 0.25) / n as f64;
                        let z = 0.5 + 29.0 * (j as f64 + 0.25) / n as f64;
                        loops.push(patch_loop(
                            b,
                            r,
                            (u, u + 0.5 * 2.8 / n as f64),
                            (z, z + 0.4 * 29.0 / n as f64),
                            true,
                        ));
                    }
                }
                if stray {
                    loops.push(patch_loop(b, r, (2.0, 2.2), (1.0, 2.0), true));
                }
                loops
            })
        };
        let body = make(false);
        // Cold: nothing remembered (other tests of this file run beside
        // it, so a face of theirs may be; not one of these).
        craft3d_topo::forget_passed();
        let started = std::time::Instant::now();
        let found = check(&body, &tol());
        let whole = started.elapsed().as_secs_f64();
        craft3d_topo::forget_passed();
        let started = std::time::Instant::now();
        let region = craft3d_topo::regionless(&body, &tol());
        let rule = started.elapsed().as_secs_f64();
        // Warm: the face passed just now and is remembered.
        let started = std::time::Instant::now();
        let again = craft3d_topo::regionless(&body, &tol());
        let warm = started.elapsed().as_secs_f64();
        assert!(again.is_empty());
        println!(
            "REGION {} holes: check {whole:.4} s, the region rule {rule:.4} s, remembered {warm:.5} s",
            n * n
        );
        assert_eq!(found, vec![], "{} holes", n * n);
        assert!(region.is_empty());
        assert!(regionless(&make(true)), "{} holes and a stray one", n * n);
    }
}

/// A face that passed is remembered by everything the rule reads of it:
/// asked again it passes, at the same work counted; the same loops on the
/// face turned over (one bit of the key) are still found, every time, and
/// a fault is never remembered.
#[test]
fn a_remembered_face_is_the_same_face_bit_for_bit() {
    use craft3d_geom::allowance;
    let r = 3.0;
    let good = sheet(r, false, |b| {
        vec![patch_loop(b, r, (-0.5, 0.5), (0.0, 1.0), false)]
    });
    let bad = sheet(r, true, |b| {
        vec![patch_loop(b, r, (-0.5, 0.5), (0.0, 1.0), false)]
    });
    let (first, cold) = allowance::measured(|| craft3d_topo::regionless(&good, &tol()));
    let (second, warm) = allowance::measured(|| craft3d_topo::regionless(&good, &tol()));
    assert!(first.is_empty() && second.is_empty());
    assert_eq!(cold, warm, "a hit charges what the check took");
    // On a spline patch, whose projections count their work: a hit
    // charges that work, not nothing.
    let patch = bump_patch();
    craft3d_topo::forget_passed();
    let (first, cold) = allowance::measured(|| craft3d_topo::regionless(&patch, &tol()));
    let (second, warm) = allowance::measured(|| craft3d_topo::regionless(&patch, &tol()));
    assert!(first.is_empty() && second.is_empty());
    assert!(cold > 0, "the check of a spline face counts its work");
    assert_eq!(cold, warm, "a hit charges what the check took");
    for _ in 0..2 {
        assert_eq!(craft3d_topo::regionless(&bad, &tol()).len(), 1);
    }
    // The good face once more, after the bad one: still a pass.
    assert!(craft3d_topo::regionless(&good, &tol()).is_empty());
}

/// A sheet of one face on a quadratic spline patch over the unit square,
/// raised in its middle, bounded by its four sides (straight: their
/// control points lie on them).
fn bump_patch() -> Body {
    use craft3d_geom::NurbsSurface;
    let mut ctrl = Vec::new();
    for i in 0..3 {
        for j in 0..3 {
            let z = if i == 1 && j == 1 { 0.3 } else { 0.0 };
            ctrl.push(Vec3::new(0.5 * i as f64, 0.5 * j as f64, z));
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let surface = NurbsSurface::new(2, 2, knots.clone(), knots, 3, 3, ctrl, vec![1.0; 9]).unwrap();
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    let v: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let mut coedges = Vec::new();
    for k in 0..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        let edge = body.add_edge(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, 1.0),
            v[k],
            v[(k + 1) % 4],
        );
        coedges.push(Coedge {
            edge,
            reversed: false,
        });
    }
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![lp]);
    body.add_shell(vec![face]);
    body
}
