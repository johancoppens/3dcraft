//! A face on a torus whose boundary runs most of the way around the axis:
//! the chart that tells inside from outside on it is the torus cut open
//! along a circle around the tube that the boundary does not cross, and
//! that circle was put where the boundary ran (the fuzzer's findings
//! 20796666, 20815546 and 20820477 of 2026-10-20: a union, an intersection
//! and a difference, right as bodies, whose points were then classified
//! wrong against them, which the boolean's own check reported). The widest
//! stretch of the circle that no piece of the boundary covers was walked
//! from the first piece's start, and a piece that started later and ran on
//! past a full turn was not counted at the start of the walk; a gap it
//! covered was taken, and the winding of the loops in a chart cut there
//! said inside where the face was not.
//!
//! The case: a ring with a dimple (a small ball taken out of the top of
//! its tube, one loop over a tenth of a turn) less a ball that reaches
//! every part of the tube but a short stretch across from its centre, so
//! that the second loop covers all but that stretch and the only circle
//! the two loops leave is in it. The oracles: every point of a grid
//! through the box is inside the result exactly when the closed forms say
//! so (in the tube, in neither ball), the boolean's own check, and a
//! further cut through the result whose two parts make it up again, at
//! scales from 1e-3 to 1e3.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract};
use craft3d_kernel::boolean::{Place, boolean, classify_points};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, check};

/// The scales these tests run at (the tolerance scales along).
const AT: [f64; 3] = [1e-3, 1.0, 1e3];

const MAJOR: f64 = 10.0;
const MINOR: f64 = 3.0;
/// The dimple: a ball of radius 1 centred on the top of the tube at angle
/// 2.5 about the axis.
const DIMPLE: f64 = 2.5;
const DIMPLE_RADIUS: f64 = 1.0;
/// The ball that reaches all round: centred 2 along x, of radius 8.98,
/// which misses the tube only within about 0.15 of the angle π.
const BALL_CENTRE: [f64; 3] = [2.0, 0.0, 0.0];
const BALL_RADIUS: f64 = 8.98;

fn dimple_centre() -> [f64; 3] {
    let (sin, cos) = DIMPLE.sin_cos();
    [MAJOR * cos, MAJOR * sin, MINOR]
}

/// Inside the exact result: in the tube, in neither ball.
fn exact(p: Vec3, k: f64) -> bool {
    let radial = p.x.hypot(p.y);
    let to_core = (radial - MAJOR * k).hypot(p.z);
    let d = dimple_centre();
    let to_dimple = p.distance(Vec3::new(d[0], d[1], d[2]) * k);
    let to_ball = p.distance(Vec3::new(BALL_CENTRE[0], BALL_CENTRE[1], BALL_CENTRE[2]) * k);
    to_core < MINOR * k && to_dimple > DIMPLE_RADIUS * k && to_ball > BALL_RADIUS * k
}

/// How far `p` is from the nearest boundary surface of the exact result,
/// so that points near one are left out of the grid.
fn clearance(p: Vec3, k: f64) -> f64 {
    let radial = p.x.hypot(p.y);
    let to_core = (radial - MAJOR * k).hypot(p.z);
    let d = dimple_centre();
    let to_dimple = p.distance(Vec3::new(d[0], d[1], d[2]) * k);
    let to_ball = p.distance(Vec3::new(BALL_CENTRE[0], BALL_CENTRE[1], BALL_CENTRE[2]) * k);
    (to_core - MINOR * k)
        .abs()
        .min((to_dimple - DIMPLE_RADIUS * k).abs())
        .min((to_ball - BALL_RADIUS * k).abs())
}

#[track_caller]
fn op(a: &Body, b: &Body, op: BooleanOp, tol: &Tolerance, what: &str) -> Body {
    let body = match boolean(a, b, op, tol) {
        Ok(r) => r.body,
        Err(e) => panic!("{what}: {op:?}: {e}"),
    };
    assert_eq!(check(&body, tol), [], "{what}: {op:?}");
    verified(a, b, op, &body, tol, what);
    body
}

/// A grid of `n³` points through the box around the ring, clear of every
/// boundary surface by a thousandth of the size.
fn grid(n: usize, k: f64) -> Vec<Vec3> {
    let reach = (MAJOR + MINOR) * k;
    let mut points = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for l in 0..n {
                let f = |i: usize| -reach + 2.0 * reach * (i as f64 + 0.5) / n as f64;
                let p = Vec3::new(f(i), f(j), f(l));
                if clearance(p, k) > 1e-3 * k {
                    points.push(p);
                }
            }
        }
    }
    points
}

#[test]
fn a_face_whose_boundary_runs_round_the_axis_is_charted_where_it_is_not() {
    for k in AT {
        let tol = tolerance(k);
        let what = format!("ring with a dimple less a ball at scale {k}");
        let ring = ring(k, [0.0, 0.0, 0.0], MAJOR, MINOR);
        let dimpled = op(
            &ring,
            &ball(k, dimple_centre(), DIMPLE_RADIUS),
            Subtract,
            &tol,
            &what,
        );
        let big = ball(k, BALL_CENTRE, BALL_RADIUS);
        let result = op(&dimpled, &big, Subtract, &tol, &what);
        // The torus face keeps two loops: the dimple's and the ball's.
        let torus_loops: Vec<usize> = result
            .faces
            .iter()
            .filter(|f| matches!(f.surface, craft3d_geom::Surface::Torus(_)))
            .map(|f| f.loops.len())
            .collect();
        assert_eq!(torus_loops, [2], "{what}: loops of the torus face");
        // Every point of the grid is placed as the closed forms say.
        let points = grid(13, k);
        let found = classify_points(&result, &points, &tol).unwrap();
        let mut wrong = Vec::new();
        for (p, place) in points.iter().zip(&found) {
            let inside = match place {
                Place::Inside => true,
                Place::Outside => false,
                other => panic!("{what}: {p:?} is {other:?}"),
            };
            if inside != exact(*p, k) {
                wrong.push(*p);
            }
        }
        assert!(
            wrong.is_empty(),
            "{what}: {} of {} points placed wrong, the first {:?}",
            wrong.len(),
            points.len(),
            wrong.first()
        );
        // A block through the tube at angle 0, where the ball cut its
        // inner side: the cut and the common part make the result up again.
        let block = boxed(k, [9.0, -5.0, -5.0], [11.0, 5.0, 5.0]);
        let cut = op(&result, &block, Subtract, &tol, &what);
        let common = op(&result, &block, Intersect, &tol, &what);
        let (v, s) = (mass(&result).volume, mass(&result).area);
        let total = mass(&cut).volume + mass(&common).volume;
        let band = 4.0 * tol.linear * (s + mass(&block).area) + 1e-9 * v;
        assert!(
            (total - v).abs() <= band,
            "{what}: {total} against {v} (band {band})"
        );
    }
}
