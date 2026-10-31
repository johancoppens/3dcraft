//! A fillet along an earlier round (an R1 blocker, 2026-10-21): "round
//! all edges of a box in two steps", first the upright edges, then the
//! rim of the top, which runs along the rounds' arcs. With the rounds'
//! own radius the ball rolling round each arc has its centre on the
//! round's axis, so the fillet there is a piece of that ball: the box
//! rounded all round in one step, held to the closed form and to the
//! one-step body at three scales and turned. A radius between half the
//! round's and the round's (a torus reaching its own axis) and one past
//! it (the ball would reach past the axis) are refused with the reason.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::fillet::{FilletEdge, fillet, tangent_chain};
use craft3d_kernel::shell::shell;
use craft3d_math::{Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

const SIZE: [f64; 3] = [10.0, 8.0, 6.0];

/// The edge whose middle is nearest `middle` (at scale 1).
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

/// The flat face whose plane passes nearest `point` (at scale 1).
fn plane_at(body: &Body, k: f64, point: [f64; 3]) -> FaceId {
    let target = Vec3::new(point[0] * k, point[1] * k, point[2] * k);
    let (index, _) = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f.surface, Surface::Plane(_)))
        .map(|(i, f)| {
            let (u, v) = f.surface.project(target).unwrap();
            (i, f.surface.eval(u, v).distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    FaceId(index as u32)
}

const UPRIGHT: [[f64; 3]; 4] = [
    [0.0, 0.0, 3.0],
    [10.0, 0.0, 3.0],
    [0.0, 8.0, 3.0],
    [10.0, 8.0, 3.0],
];

/// The block with its four upright edges rounded with radius `r`.
fn rounded_upright(k: f64, r: f64) -> Body {
    let block = boxed(k, [0.0; 3], SIZE);
    let edges: Vec<FilletEdge> = UPRIGHT
        .iter()
        .map(|m| FilletEdge {
            edge: edge_at(&block, k, *m),
            radius: r * k,
        })
        .collect();
    fillet(&block, &edges, &tolerance(k)).unwrap().body
}

/// The rim of the top of `body`: the chain of edges round it (four lines
/// and the four arcs where the upright rounds meet the top).
fn top_rim(body: &Body, k: f64, radius: f64) -> Vec<FilletEdge> {
    let front = edge_at(body, k, [5.0, 0.0, 6.0]);
    let top = plane_at(body, k, [5.0, 4.0, 6.0]);
    let chain = tangent_chain(body, front, top).unwrap().unwrap();
    assert_eq!(chain.len(), 8, "four lines and four arcs");
    chain
        .into_iter()
        .map(|(edge, _)| FilletEdge {
            edge,
            radius: radius * k,
        })
        .collect()
}

/// The block `w × d × h` with its four upright edges and the four edges
/// of its top rounded with radius `r`: the rounds of the upright edges,
/// the straight rounds of the top between them, and at each top corner
/// the column of an upright round less the eighth of the ball in it.
fn closed_form([w, d, h]: [f64; 3], r: f64) -> f64 {
    w * d * h
        - (4.0 - PI) * r * r * h
        - 2.0 * (w + d - 4.0 * r) * (1.0 - PI / 4.0) * r * r
        - 4.0 * (PI * r.ppowi(3) / 4.0 - PI * r.ppowi(3) / 6.0)
}

#[test]
fn the_top_rim_rounded_along_upright_rounds_of_its_own_radius_is_the_box_rounded_in_one_step() {
    let r = 2.0;
    let turn = Transform::rotation(Vec3::new(0.3, 1.0, -0.8), 2.2).unwrap();
    for k in SCALES {
        let tol = tolerance(k);
        let first = rounded_upright(k, r);
        let made = fillet(&first, &top_rim(&first, k, r), &tol)
            .unwrap_or_else(|e| panic!("at scale {k}: {e}"));
        assert_eq!(check(&made.body, &tol), [], "at scale {k}");
        let m = mass(&made.body);
        assert_close(m.volume, closed_form(SIZE, r) * k * k * k, "two steps");
        // The same body in one step: the eight edges at once, with a
        // ball in each top corner.
        let block = boxed(k, [0.0; 3], SIZE);
        let mut middles: Vec<[f64; 3]> = UPRIGHT.to_vec();
        middles.extend([
            [5.0, 0.0, 6.0],
            [5.0, 8.0, 6.0],
            [0.0, 4.0, 6.0],
            [10.0, 4.0, 6.0],
        ]);
        let edges: Vec<FilletEdge> = middles
            .iter()
            .map(|m| FilletEdge {
                edge: edge_at(&block, k, *m),
                radius: r * k,
            })
            .collect();
        let one = fillet(&block, &edges, &tol).unwrap().body;
        let o = mass(&one);
        assert_close(m.volume, o.volume, "one step");
        assert_close(m.area, o.area, "the area of one step");
        // A piece of a ball of the radius in each corner, about a point
        // on the axis of the round below it.
        let balls: Vec<Vec3> = made
            .body
            .faces
            .iter()
            .filter_map(|f| match &f.surface {
                Surface::Sphere(s) => {
                    assert_close(s.radius(), r * k, "the ball's radius");
                    Some(s.frame().origin())
                }
                _ => None,
            })
            .collect();
        assert_eq!(balls.len(), 4);
        for centre in balls {
            let x = if centre.x < 5.0 * k { r } else { 10.0 - r };
            let y = if centre.y < 4.0 * k { r } else { 8.0 - r };
            let want = Vec3::new(x * k, y * k, (6.0 - r) * k);
            assert!(centre.distance(want) <= tol.linear, "{centre:?}");
        }
        // Turned off every axis: the same volume, to the tolerance.
        let turned = first.transformed(&turn).unwrap();
        let rim: Vec<FilletEdge> = top_rim(&first, k, r);
        let made =
            fillet(&turned, &rim, &tol).unwrap_or_else(|e| panic!("turned, at scale {k}: {e}"));
        assert_eq!(check(&made.body, &tol), [], "turned, at scale {k}");
        let v = mass(&made.body).volume;
        let off = v - closed_form(SIZE, r) * k * k * k;
        assert!(
            off.abs() <= tol.linear * m.area,
            "turned, at scale {k}: off by {off:e}"
        );
    }
}

/// The two-step box shelled as thick as its rounds: the rounds and the
/// balls of the second step vanish together, the hollow is the box
/// shrunk by the wall (`shell`, the R1 enclosure).
#[test]
fn the_box_rounded_in_two_steps_shells_as_thick_as_its_rounds() {
    let r = 2.0;
    for k in SCALES {
        let tol = tolerance(k);
        let first = rounded_upright(k, r);
        let rounded = fillet(&first, &top_rim(&first, k, r), &tol).unwrap().body;
        let bottom = plane_at(&rounded, k, [5.0, 4.0, 0.0]);
        for t in [2.0, 2.5] {
            let made = shell(&rounded, t * k, &[bottom], &tol)
                .unwrap_or_else(|e| panic!("shell {t} at scale {k}: {e}"));
            assert_eq!(check(&made.body, &tol), [], "shell {t} at scale {k}");
            let hollow = (10.0 - 2.0 * t) * (8.0 - 2.0 * t) * (6.0 - t);
            assert_close(
                mass(&made.body).volume,
                (closed_form(SIZE, r) - hollow) * k * k * k,
                "the shell",
            );
        }
    }
}

/// Radii the rim along a round of radius 2 is refused with: between half
/// the round's and the round's (the torus would reach its own axis), and
/// past the round's (the ball would reach past its axis). Below half it
/// is a torus, as before.
#[test]
fn other_radii_along_the_round_are_refused_with_the_reason_or_built() {
    let k = 1.0;
    let tol = tolerance(k);
    let first = rounded_upright(k, 2.0);
    // The rim is a circle of the round's radius 2 between the flat top
    // and the upright round: the ball's centre is 2 - r from the axis,
    // as far as the radius at r = 1, on the axis at r = 2 (closed form;
    // until 2026-10-22 a text without the numbers).
    match fillet(&first, &top_rim(&first, k, 1.5), &tol) {
        Err(KernelError::FilletSpindle {
            radius, below, own, ..
        }) => {
            assert_close(radius, 1.5 * k, "radius");
            assert_close(below, k, "what fits below");
            assert_close(own, 2.0 * k, "the round's own radius");
        }
        other => panic!("1.5: expected the spindle refusal, got {other:?}"),
    }
    // Past the rounds' radius the ball would reach past their axes: too
    // large, and the largest that fits is the rounds' own radius, which
    // builds.
    for radius in [2.5, 3.0] {
        match fillet(&first, &top_rim(&first, k, radius), &tol) {
            Err(KernelError::FilletTooLarge { largest, .. }) => {
                assert_close(largest, 2.0 * k, "what fits");
            }
            other => panic!("{radius}: expected too large, got {other:?}"),
        }
    }
    let made = fillet(&first, &top_rim(&first, k, 0.5), &tol).unwrap();
    assert_eq!(check(&made.body, &tol), []);
    let tori = made
        .body
        .faces
        .iter()
        .filter(|f| matches!(f.surface, Surface::Torus(_)))
        .count();
    assert_eq!(tori, 4);
}

/// Radii within the tolerance of the rounds' radius (the review of
/// 2026-10-22): each either builds, as the round's own radius does, or is
/// refused with something true. Above it "too large" names the round's
/// radius as what fits, and that builds (before: a search's 0.9977, below
/// half of it, while 2 builds); below it the spindle refusal, which names
/// the rim's own radius, and that builds. At three scales, with offsets
/// from a few ulps to past the tolerance on both sides.
#[test]
fn radii_within_the_tolerance_of_the_rounds_build_or_say_what_fits() {
    let r = 2.0;
    for k in SCALES {
        let tol = tolerance(k);
        let first = rounded_upright(k, r);
        let builds = |radius: f64| {
            let made = fillet(&first, &top_rim(&first, k, radius / k), &tol)
                .unwrap_or_else(|e| panic!("{radius} at scale {k} should build: {e}"));
            assert_eq!(check(&made.body, &tol), [], "{radius} at scale {k}");
            let m = mass(&made.body);
            let off = m.volume - closed_form(SIZE, r) * k * k * k;
            assert!(
                off.abs() <= tol.linear * m.area,
                "{radius} at scale {k}: off by {off:e}"
            );
        };
        // Offsets as fractions of the tolerance at scale 1 (1e-6).
        for d in [
            1e-12, 1e-9, 1e-7, 4e-7, 5e-7, 9e-7, 1.1e-6, -1e-12, -1e-9, -5e-7, -9e-7, -1.1e-6,
        ] {
            let asked = (r + d) * k;
            match fillet(&first, &top_rim(&first, k, r + d), &tol) {
                Ok(made) => {
                    assert_eq!(check(&made.body, &tol), [], "{d:e} at scale {k}");
                    let m = mass(&made.body);
                    let off = m.volume - closed_form(SIZE, r) * k * k * k;
                    assert!(
                        off.abs() <= tol.linear * m.area,
                        "{d:e} at scale {k}: off by {off:e}"
                    );
                }
                Err(KernelError::FilletTooLarge { largest, .. }) => {
                    assert!(largest < asked, "{d:e} at scale {k}: {largest} of {asked}");
                    assert!(
                        (largest - r * k).abs() <= tol.linear,
                        "{d:e} at scale {k}: what fits is {largest}"
                    );
                    builds(largest);
                }
                // The spindle refusal names the round's own radius as
                // one that builds (until 2026-10-22 in words only).
                Err(KernelError::FilletSpindle { own, .. }) => {
                    assert!(asked < r * k, "{d:e} at scale {k}: {asked}");
                    assert!(
                        (own - r * k).abs() <= tol.linear,
                        "{d:e} at scale {k}: names {own}"
                    );
                    builds(r * k);
                }
                Err(error) => panic!("{d:e} at scale {k}: {error}"),
            }
        }
    }
}
