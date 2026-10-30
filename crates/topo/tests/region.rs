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
