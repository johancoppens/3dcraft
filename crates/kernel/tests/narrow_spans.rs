//! A spline with a knot span far narrower than its neighbours (1e-10
//! among spans of a twelfth or a 24th, as the joints of a loft's sections
//! that nearly meet make them: the wavy-vase Jig, 2026-10-31), cubic and
//! of degree twelve with every inner knot repeated, swept into the wall of
//! a solid: the wall is meshed at a thousandth of the body's size, with a
//! grid of a few dozen cells along it (the Bézier bounds alone asked for
//! 1600 to 10000, beyond the limit; `tess` face.rs
//! `a_span_far_narrower_than_its_neighbours_does_not_set_the_grid`), and
//! the mesh is within the chordal tolerance of the surface, measured at
//! points spread over every triangle, at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::line;
use craft3d_geom::Surface;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::check;

type P = [f64; 2];

const NARROW: f64 = 1e-10;

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The wave, at `t` in `0..1`, and its derivative.
fn wave(t: f64) -> (P, P) {
    let w = 6.0 * std::f64::consts::PI;
    (
        [-6.0 + 12.0 * t, 2.0 + 0.5 * (w * t).psin()],
        [12.0, 0.5 * w * (w * t).pcos()],
    )
}

/// Cubic: 24 even spans and one more knot `NARROW` after the middle,
/// control points on the wave at their Greville abscissae.
fn narrow_cubic() -> (usize, Vec<f64>, Vec<P>) {
    let mut knots = vec![0.0; 4];
    knots.extend((1..24).map(|i| f64::from(i) / 24.0));
    knots.push(0.5 + NARROW);
    knots.sort_by(f64::total_cmp);
    knots.extend([1.0; 4]);
    let count = knots.len() - 4;
    let points = (0..count)
        .map(|i| wave(knots[i + 1..=i + 3].iter().sum::<f64>() / 3.0).0)
        .collect();
    (3, knots, points)
}

/// Degree twelve, twelve even pieces, the middle one cut `NARROW` after
/// its start, each the Hermite cubic of the wave raised to degree twelve
/// and joined with their tangents; every inner knot `times` times. Twelve
/// times (every joint a knot a profile cuts at, as the sections of a loft
/// get them) the piece 1e-10 long is shorter than the tolerance, and is
/// kept inside its neighbours, which meet it in tangents (until
/// 2026-11-01 it failed as a branching profile); eleven times the joint
/// point drops out and the curve is one spline.
fn narrow_twelve(times: usize) -> (usize, Vec<f64>, Vec<P>) {
    let mut breaks: Vec<f64> = (0..=12).map(|i| f64::from(i) / 12.0).collect();
    breaks.push(0.5 + NARROW);
    breaks.sort_by(f64::total_cmp);
    let mut points: Vec<P> = Vec::new();
    let mut knots = vec![0.0; 13];
    for (n, pair) in breaks.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let ((pa, da), (pb, db)) = (wave(a), wave(b));
        let h = (b - a) / 3.0;
        let mut piece = vec![
            pa,
            [pa[0] + h * da[0], pa[1] + h * da[1]],
            [pb[0] - h * db[0], pb[1] - h * db[1]],
            pb,
        ];
        while piece.len() < 13 {
            let up = piece.len() as f64;
            let mut raised = vec![piece[0]];
            for i in 1..piece.len() {
                let r = i as f64 / up;
                raised.push([
                    r * piece[i - 1][0] + (1.0 - r) * piece[i][0],
                    r * piece[i - 1][1] + (1.0 - r) * piece[i][1],
                ]);
            }
            raised.push(*piece.last().unwrap());
            piece = raised;
        }
        // Eleven times, the joint point lies between its neighbours (the
        // pieces are tangent in the parameter): no control point.
        if n > 0 && times == 11 {
            points.pop();
        }
        points.extend(piece.into_iter().skip(usize::from(n > 0)));
        if n + 2 < breaks.len() {
            knots.extend(std::iter::repeat_n(b, times));
        }
    }
    knots.extend([1.0; 13]);
    (12, knots, points)
}

#[test]
fn a_wall_with_a_span_far_narrower_than_its_neighbours_meshes_within_the_tolerance() {
    for k in [1e-3, 1.0, 1e6] {
        for (name, (degree, knots, points)) in [
            ("cubic", narrow_cubic()),
            ("twelve, knots eleven times", narrow_twelve(11)),
            ("twelve, knots twelve times", narrow_twelve(12)),
        ] {
            let scaled: Vec<P> = points.iter().map(|p| [p[0] * k, p[1] * k]).collect();
            let (first, last) = (scaled[0], scaled[scaled.len() - 1]);
            let curves = vec![
                SketchCurve {
                    id: 1,
                    curve: Curve2::Spline(Spline2 {
                        degree,
                        knots: knots.clone(),
                        control: scaled.clone(),
                    }),
                },
                line(2, last, [6.0 * k, 8.0 * k]),
                line(3, [6.0 * k, 8.0 * k], [-6.0 * k, 8.0 * k]),
                line(4, [-6.0 * k, 8.0 * k], first),
            ];
            let profiles = detect_profiles(&curves, &tolerance(k))
                .unwrap_or_else(|e| panic!("{name} at {k}: {e:?}"));
            let made = extrude(&profiles, &Frame::WORLD, 0.0, 5.0 * k, &tolerance(k)).unwrap();
            let body = &made.body;
            assert_eq!(check(body, &tolerance(k)), vec![]);
            let wall = body
                .faces
                .iter()
                .position(|f| matches!(f.surface, Surface::Nurbs(_)))
                .unwrap();
            let chord = 1e-3 * body.bbox().unwrap().diagonal();
            let mesh = tessellate_body(
                body,
                &TessParams {
                    chordal_tolerance: chord,
                    ..TessParams::default()
                },
            )
            .unwrap();
            let surface = &body.faces[wall].surface;
            let mut on_wall = 0_usize;
            let mut worst = 0.0_f64;
            let mut found: Vec<Option<(f64, f64)>> = vec![None; mesh.positions.len()];
            const N: u32 = 6;
            for (t, triangle) in mesh.triangles.iter().enumerate() {
                if mesh.face_ids[t] as usize != wall {
                    continue;
                }
                on_wall += 1;
                let corners = triangle.map(|v| {
                    let p = mesh.positions[v as usize];
                    let uv = *found[v as usize].get_or_insert_with(|| surface.project(p).unwrap());
                    (p, uv)
                });
                let [(a, _), (b, _), (c, _)] = corners;
                for i in 0..=N {
                    for j in 0..=N - i {
                        let (s, r) = (f64::from(i) / f64::from(N), f64::from(j) / f64::from(N));
                        let p = a * (1.0 - s - r) + b * s + c * r;
                        let nearest = corners
                            .iter()
                            .filter_map(|&(_, start)| surface.project_from(p, start).ok())
                            .map(|(u, v)| surface.eval(u, v).distance(p))
                            .fold(f64::INFINITY, f64::min);
                        worst = worst.max(nearest);
                    }
                }
            }
            assert!(
                worst <= chord * (1.0 + 1e-6),
                "{name} at {k}: {worst} from the surface, chord {chord}"
            );
            // A few dozen cells along, not the thousands of the Bézier bounds.
            assert!(
                on_wall > 0 && on_wall < 4000,
                "{name} at {k}: {on_wall} triangles"
            );
        }
    }
}

/// A spline whose piece shorter than the tolerance turns a corner (its
/// neighbours leave it at right angles) is refused by name, with the knot
/// it starts at, at three scales: no smooth piece can hold it.
#[test]
fn a_piece_shorter_than_the_tolerance_at_a_corner_is_refused_by_name() {
    for k in [1e-3, 1.0, 1e6] {
        let s = |p: P| [p[0] * k, p[1] * k];
        // Along x to the origin, a step of 1e-10 up, then up to y = 6.
        let control: Vec<P> = [
            [-6.0, 0.0],
            [-4.0, 0.0],
            [-2.0, 0.0],
            [0.0, 0.0],
            [0.0, 1e-11],
            [0.0, 2e-11],
            [0.0, 3e-11],
            [0.0, 2.0],
            [0.0, 4.0],
            [0.0, 6.0],
        ]
        .iter()
        .map(|p| s(*p))
        .collect();
        let knots = vec![
            0.0,
            0.0,
            0.0,
            0.0,
            0.5,
            0.5,
            0.5,
            0.5 + NARROW,
            0.5 + NARROW,
            0.5 + NARROW,
            1.0,
            1.0,
            1.0,
            1.0,
        ];
        let curves = vec![
            SketchCurve {
                id: 7,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots,
                    control,
                }),
            },
            line(2, s([0.0, 6.0]), s([-6.0, 6.0])),
            line(3, s([-6.0, 6.0]), s([-6.0, 0.0])),
        ];
        match detect_profiles(&curves, &tolerance(k)) {
            Err(craft3d_kernel::KernelError::ShortSplinePiece { id: 7, knot }) => {
                assert_eq!(knot, 0.5, "at {k}");
            }
            other => panic!("at {k}: {other:?}"),
        }
    }
    let message = craft3d_kernel::KernelError::ShortSplinePiece { id: 7, knot: 0.5 }.to_string();
    assert!(
        message.contains("a piece of spline 7 shorter than the tolerance at knot 0.5"),
        "{message}"
    );
}
