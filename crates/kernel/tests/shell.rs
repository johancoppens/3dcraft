//! Shells against closed forms (docs/testing.md, oracle 1): every result
//! passes the checker and has the volume, area and counts that follow
//! from the thickness, at three scales.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, rod, solve, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::shell::{Shelled, Skin, shell};
use craft3d_math::Vec3;
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The face on a surface of `kind` nearest `point` (for scale 1).
fn face_at(body: &Body, k: f64, kind: &str, point: [f64; 3]) -> FaceId {
    let target = Vec3::new(point[0] * k, point[1] * k, point[2] * k);
    let (index, _) = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face.surface.kind() == kind)
        .map(|(i, face)| {
            let (u, v) = face.surface.project(target).unwrap();
            (i, face.surface.eval(u, v).distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    FaceId(index as u32)
}

fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

#[track_caller]
fn hollowed(body: &Body, thickness: f64, open: &[FaceId], k: f64) -> Shelled {
    let tol = tolerance(k);
    let made = match shell(body, thickness * k, open, &tol) {
        Ok(made) => made,
        Err(e) => panic!("shell at scale {k}: {e}"),
    };
    assert_eq!(check(&made.body, &tol), [], "at scale {k}");
    assert_eq!(made.faces.len(), made.body.faces.len());
    assert_eq!(made.edges.len(), made.body.edges.len());
    assert_eq!(made.vertices.len(), made.body.vertices.len());
    made
}

#[track_caller]
fn assert_mass(made: &Shelled, volume: f64, area: f64, k: f64) {
    let m = mass(&made.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

fn count(made: &Shelled, which: Skin) -> usize {
    made.faces.iter().filter(|(skin, _)| *skin == which).count()
}

fn refused(body: &Body, thickness: f64, open: &[FaceId], k: f64) -> KernelError {
    match shell(body, thickness * k, open, &tolerance(k)) {
        Ok(_) => panic!("a shell of {thickness} at scale {k} was made"),
        Err(e) => e,
    }
}

#[test]
fn an_open_box_and_a_closed_hollow_block() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
        let top = face_at(&block, k, "plane", [5.0, 4.0, 6.0]);
        let made = hollowed(&block, 1.0, &[top], k);
        // The hollow is 8 x 6 x 5; the rim is what is left of the top.
        assert_mass(&made, 480.0 - 240.0, 376.0 - 48.0 + 48.0 + 140.0, k);
        assert_eq!(made.body.shells.len(), 1);
        assert_eq!(count(&made, Skin::Outer), 6);
        assert_eq!(count(&made, Skin::Inner), 5);
        assert!(!made.faces.contains(&(Skin::Inner, top)));

        let made = hollowed(&block, 1.0, &[], k);
        // A cavity of 8 x 6 x 4 inside, a shell of its own.
        assert_mass(&made, 480.0 - 192.0, 376.0 + 2.0 * (48.0 + 32.0 + 24.0), k);
        assert_eq!(made.body.shells.len(), 2);
        assert_eq!(count(&made, Skin::Inner), 6);
    }
}

#[test]
fn a_cup_and_a_tube() {
    for k in SCALES {
        let body = rod(k, [0.0, 0.0, 0.0], 3.0, 5.0);
        let lid = face_at(&body, k, "plane", [0.0, 0.0, 5.0]);
        let floor = face_at(&body, k, "plane", [0.0, 0.0, 0.0]);
        let made = hollowed(&body, 0.5, &[lid], k);
        let (r, i) = (3.0, 2.5);
        assert_mass(
            &made,
            PI * r * r * 5.0 - PI * i * i * 4.5,
            2.0 * PI * r * 5.0
                + PI * r * r
                + PI * (r * r - i * i)
                + 2.0 * PI * i * 4.5
                + PI * i * i,
            k,
        );
        let made = hollowed(&body, 0.5, &[lid, floor], k);
        assert_mass(
            &made,
            PI * (r * r - i * i) * 5.0,
            2.0 * PI * (r + i) * 5.0 + 2.0 * PI * (r * r - i * i),
            k,
        );
        assert_eq!(made.body.faces.len(), 4);
        // Half the radius is all the wall there is.
        match refused(&body, 3.0, &[lid], k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 2.99 * k && largest < 3.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

#[test]
fn an_l_shaped_prism_open_at_one_end() {
    for k in SCALES {
        let a = boxed(k, [0.0, 0.0, 0.0], [10.0, 4.0, 6.0]);
        let b = boxed(k, [0.0, 0.0, 0.0], [4.0, 10.0, 6.0]);
        let ell = solve(&a, &b, BooleanOp::Union, k).body;
        assert_eq!(ell.faces.len(), 8);
        let top = face_at(&ell, k, "plane", [2.0, 2.0, 6.0]);
        let made = hollowed(&ell, 1.0, &[top], k);
        // The section of the L is 64; that of the hollow 8 x 2 + 2 x 6.
        let (outer, inner) = (64.0, 28.0);
        let outline = 2.0 * (10.0 + 10.0);
        let inner_outline = 2.0 * (8.0 + 8.0);
        assert_mass(
            &made,
            outer * 6.0 - inner * 5.0,
            outer + outline * 6.0 + (outer - inner) + inner + inner_outline * 5.0,
            k,
        );
        // Half the narrow width: the hollow would have no width.
        match refused(&ell, 2.5, &[top], k) {
            KernelError::FaceOpTooFar { largest, .. } => {
                assert!(largest > 1.99 * k && largest < 2.0 * k, "{largest}");
            }
            other => panic!("{other}"),
        }
    }
}

/// A block of 10 x 8 x 6 whose four upright edges are rounded with
/// radius 2.
fn rounded(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
    let edges: Vec<FilletEdge> = [
        [0.0, 0.0, 3.0],
        [10.0, 0.0, 3.0],
        [0.0, 8.0, 3.0],
        [10.0, 8.0, 3.0],
    ]
    .into_iter()
    .map(|middle| FilletEdge {
        edge: edge_at(&block, k, middle),
        radius: 2.0 * k,
    })
    .collect();
    fillet(&block, &edges, &tolerance(k)).unwrap().body
}

/// The area of a w x h rectangle with corners rounded by `r`.
fn rounded_area(w: f64, h: f64, r: f64) -> f64 {
    w * h - (4.0 - PI) * r * r
}

fn rounded_outline(w: f64, h: f64, r: f64) -> f64 {
    2.0 * (w + h) - 8.0 * r + 2.0 * PI * r
}

#[test]
fn a_rounded_box_has_inner_rounds_of_the_radius_less_the_wall() {
    for k in SCALES {
        let body = rounded(k);
        let top = face_at(&body, k, "plane", [5.0, 4.0, 6.0]);
        for t in [0.5, 1.0, 1.5] {
            let made = hollowed(&body, t, &[top], k);
            let (outer, inner) = (
                rounded_area(10.0, 8.0, 2.0),
                rounded_area(10.0 - 2.0 * t, 8.0 - 2.0 * t, 2.0 - t),
            );
            let h = 6.0 - t;
            assert_mass(
                &made,
                outer * 6.0 - inner * h,
                2.0 * outer - inner
                    + rounded_outline(10.0, 8.0, 2.0) * 6.0
                    + inner
                    + rounded_outline(10.0 - 2.0 * t, 8.0 - 2.0 * t, 2.0 - t) * h,
                k,
            );
            // The inner rounds are cylinders of radius 2 - t.
            let radii: Vec<f64> = made
                .faces
                .iter()
                .enumerate()
                .filter(|(_, (skin, _))| *skin == Skin::Inner)
                .filter_map(|(i, _)| match &made.body.faces[i].surface {
                    craft3d_geom::Surface::Cylinder(c) => Some(c.radius()),
                    _ => None,
                })
                .collect();
            assert_eq!(radii.len(), 4);
            for r in radii {
                assert_close(r, (2.0 - t) * k, "inner radius");
            }
        }
        // At the radius of the rounds the inner rounds are gone: refused
        // with what fits.
        for t in [2.0, 2.5] {
            match refused(&body, t, &[top], k) {
                KernelError::FaceOpTooFar { largest, faces, .. } => {
                    assert!(largest > 1.99 * k && largest < 2.0 * k, "{largest}");
                    assert!(!faces.is_empty());
                }
                other => panic!("{other}"),
            }
        }
    }
}

#[test]
fn a_hole_through_the_open_side_and_a_boss_on_the_closed_one() {
    for k in SCALES {
        // A hole of radius 1 through a block of 10 x 10 x 6, open at the
        // top: a tube of wall 1 stands in the hollow round the hole.
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 6.0]);
        let drill = rod(k, [5.0, 5.0, -1.0], 1.0, 8.0);
        let drilled = solve(&block, &drill, BooleanOp::Subtract, k).body;
        let top = face_at(&drilled, k, "plane", [0.5, 0.5, 6.0]);
        let made = hollowed(&drilled, 1.0, &[top], k);
        assert_mass(
            &made,
            (100.0 - PI) * 6.0 - (64.0 - PI * 4.0) * 5.0,
            mass(&drilled).area / (k * k) - (100.0 - PI)
                + (36.0 + 3.0 * PI)
                + (64.0 - 4.0 * PI)
                + 32.0 * 5.0
                + 2.0 * PI * 2.0 * 5.0,
            k,
        );
        // A boss of radius 2 and height 3 on the top of a block of
        // 10 x 10 x 5, open at the bottom: the boss is a cup upside down,
        // and the hollow is 4 high inside the block.
        let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 10.0, 5.0]);
        let boss = rod(k, [5.0, 5.0, 5.0], 2.0, 3.0);
        let bossed = solve(&block, &boss, BooleanOp::Union, k).body;
        let bottom = face_at(&bossed, k, "plane", [5.0, 5.0, 0.0]);
        let made = hollowed(&bossed, 1.0, &[bottom], k);
        assert_mass(
            &made,
            500.0 + PI * 4.0 * 3.0 - (64.0 * 4.0 + PI * 3.0),
            mass(&bossed).area / (k * k) - 100.0
                + 36.0
                + (64.0 - PI)
                + 32.0 * 4.0
                + 2.0 * PI * 3.0
                + PI,
            k,
        );
    }
}

#[test]
fn what_is_not_a_shell_is_an_error() {
    let k = 1.0;
    let block = boxed(k, [0.0, 0.0, 0.0], [10.0, 8.0, 6.0]);
    let tol = tolerance(k);
    for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            shell(&block, t, &[], &tol),
            Err(KernelError::InvalidSize { .. })
        ));
    }
    assert!(matches!(
        shell(&block, 1.0, &[FaceId(99)], &tol),
        Err(KernelError::FaceOp { .. })
    ));
    assert!(matches!(
        shell(&block, 1.0, &[FaceId(1), FaceId(1)], &tol),
        Err(KernelError::FaceOp { .. })
    ));
    let all: Vec<FaceId> = (0..6).map(FaceId).collect();
    assert!(matches!(
        shell(&block, 1.0, &all, &tol),
        Err(KernelError::FaceOp { .. })
    ));
    // Half the smallest size: no hollow is left.
    match shell(&block, 3.0, &[], &tol) {
        Err(KernelError::FaceOpTooFar { largest, .. }) => {
            assert!(largest > 2.99 && largest < 3.0, "{largest}");
        }
        other => panic!("{other:?}"),
    }
}
