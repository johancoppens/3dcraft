//! The bounds the bend sequence check rests on
//! (docs/design/sheetmetal.md › *Bend sequence*), held to closed forms at
//! 1e-3, 1 and 1e6:
//!
//! - a flat's hull points lie in its outline, whatever way the outline
//!   lies in the pattern's axes (a triangle, a rotated rectangle, an
//!   arch): the box in the pattern's axes stood past a slanted bend line,
//!   so a flat lying on the die could never be proven above it;
//! - the lowest point of a built bend under a linear height is exact
//!   where its skin touches tangentially, `−(R + T)` from the axis, where
//!   the hull's tangent apexes lie below it;
//! - an arched flat's lowest point is the least of the arc's samples.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::sequence::{PieceOf, final_worlds, piece_lowest, piece_points};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, FlangeSpec, LengthFrom, On, Part, Spec, build,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn lines(pts: &[[f64; 2]], k: f64) -> Vec<SketchCurve> {
    (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect()
}

fn one(curves: &[SketchCurve], k: f64) -> Profile {
    let mut found = detect_profiles(curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    found.remove(0)
}

fn flange(edge: u64, r: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(edge),
        length: 0.0,
        angle: FRAC_PI_2,
        direction: Direction::Up,
        radius: r,
        k: 0.5,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

fn part(base: Profile, t: f64, flanges: Vec<FlangeSpec>, k: f64) -> Part {
    build(
        &Spec {
            base,
            plane: Frame::WORLD,
            thickness: t,
            flanges,
            relief: None,
            corner: Closing::OPEN,
            corners: Vec::new(),
            cutouts: Vec::new(),
            ids: Vec::new(),
        },
        &tolerance(k),
    )
    .unwrap()
}

/// Whether `p` lies in the convex polygon `poly` (either way round),
/// within `slack`.
fn in_convex(poly: &[[f64; 2]], p: [f64; 2], slack: f64) -> bool {
    let area: f64 = (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let turn = area.signum();
    (0..poly.len()).all(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let cross = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
        turn * cross >= -slack * dx.hypot(dy)
    })
}

/// The flat pattern's points of flat `f`'s hull, in its own axes.
fn hull_in_flat(part: &Part, f: usize) -> Vec<Vec3> {
    let back = final_worlds(part)[f].inverse().unwrap();
    piece_points(part, PieceOf::Flat(f))
        .into_iter()
        .map(|p| back.point(p))
        .collect()
}

#[test]
fn a_flats_hull_lies_in_its_outline_on_a_slanted_bend_line() {
    // A triangle, and a rectangle turned 30 degrees, each with a flange
    // on a slanted edge: the base's (and the flange's) hull points stay
    // in the outline, none past the bend's tangent line.
    let (c, s) = ((PI / 6.0).cos(), (PI / 6.0).sin());
    let turned: Vec<[f64; 2]> = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
        .iter()
        .map(|p| [c * p[0] - s * p[1], s * p[0] + c * p[1]])
        .collect();
    let shapes: [(&[[f64; 2]], u64); 2] =
        [(&[[0.0, 0.0], [40.0, 0.0], [20.0, 30.0]], 2), (&turned, 2)];
    for k in SCALES {
        let tol = tolerance(k);
        let t = 1.0 * k;
        for (pts, edge) in shapes {
            let mut f = flange(edge, 1.0 * k);
            f.length = 10.0 * k;
            let p = part(one(&lines(pts, k), k), t, vec![f], k);
            let scaled: Vec<[f64; 2]> = pts.iter().map(|q| [q[0] * k, q[1] * k]).collect();
            let hull = hull_in_flat(&p, 0);
            assert!(!hull.is_empty());
            for q in &hull {
                assert!(
                    in_convex(&scaled, [q.x, q.y], tol.linear),
                    "scale {k}: hull point {q:?} outside the base {scaled:?}"
                );
                assert!(q.z >= -tol.linear && q.z <= t + tol.linear);
            }
            // Nothing across the bend's tangent line, on the child's side.
            let b = &p.bends[0];
            for q in &hull {
                let across = (q.x - b.start[0]) * b.across[0] + (q.y - b.start[1]) * b.across[1];
                assert!(
                    across <= tol.linear,
                    "scale {k}: {across} past the bend line"
                );
            }
            // The flange's own flat: its hull in its outline too.
            let flat = &p.flats[1];
            let outline: Vec<[f64; 2]> = flat.outer.iter().map(|s| s.start).collect();
            for q in hull_in_flat(&p, 1) {
                assert!(
                    in_convex(&outline, [q.x, q.y], tol.linear),
                    "scale {k}: flange hull point {q:?} outside {outline:?}"
                );
            }
        }
    }
}

#[test]
fn a_bends_lowest_point_is_exact_where_its_skin_touches_tangentially() {
    for k in SCALES {
        let (t, r) = (1.0 * k, 2.0 * k);
        let mut f = flange(2, r);
        f.length = 10.0 * k;
        let p = part(
            one(
                &lines(&[[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]], k),
                k,
            ),
            t,
            vec![f],
            k,
        );
        let b = p.bends[0];
        let back = final_worlds(&p)[b.parent].inverse().unwrap();
        let zc = t + b.radius;
        for alpha in [0.3_f64, 0.7, 1.2] {
            // A height whose level planes touch the outer skin at the
            // angle `alpha` of the bend: `cos α w − sin α u` from the axis
            // (`u` across, `w` up), least `−(R + T)` there.
            let height = |x: Vec3| {
                let q = back.point(x);
                let u = (q.x - b.start[0]) * b.across[0] + (q.y - b.start[1]) * b.across[1];
                alpha.cos() * (q.z - zc) - alpha.sin() * u
            };
            let exact = -(b.radius + t);
            let got = piece_lowest(&p, PieceOf::Bend(0), height).unwrap();
            assert!(
                (got - exact).abs() <= 1e-12 * k * 10.0,
                "scale {k}, α {alpha}: lowest {got} against {exact}"
            );
            // The hull's apexes lie below the skin: by them a skin touching
            // a plane could not be proven on it.
            let by_hull = piece_points(&p, PieceOf::Bend(0))
                .into_iter()
                .map(height)
                .fold(f64::INFINITY, f64::min);
            assert!(
                by_hull < exact - 1e-3 * k,
                "scale {k}, α {alpha}: hull {by_hull} against {exact}"
            );
        }
    }
}

#[test]
fn an_arched_flats_lowest_point_is_its_arcs_least() {
    for k in SCALES {
        // A D: the bottom edge, its sides and a half circle on top.
        let (w, h) = (20.0 * k, 10.0 * k);
        let mut curves = vec![SketchCurve {
            id: 1,
            curve: Curve2::Line {
                start: [0.0, 0.0],
                end: [w, 0.0],
            },
        }];
        curves.push(SketchCurve {
            id: 2,
            curve: Curve2::Line {
                start: [w, 0.0],
                end: [w, h],
            },
        });
        curves.push(SketchCurve {
            id: 3,
            curve: Curve2::Arc {
                center: [0.5 * w, h],
                start: [w, h],
                end: [0.0, h],
            },
        });
        curves.push(SketchCurve {
            id: 4,
            curve: Curve2::Line {
                start: [0.0, h],
                end: [0.0, 0.0],
            },
        });
        let t = 1.0 * k;
        let mut f = flange(1, 1.0 * k);
        f.length = 5.0 * k;
        let p = part(one(&curves, k), t, vec![f], k);
        let back = final_worlds(&p)[0].inverse().unwrap();
        for beta in [0.4_f64, 1.1, 2.0, 2.9] {
            // Least in the direction (cos β, sin β): on the arc where it
            // points against it.
            let height = |x: Vec3| {
                let q = back.point(x);
                -(beta.cos() * q.x + beta.sin() * q.y) + 0.25 * q.z
            };
            let got = piece_lowest(&p, PieceOf::Flat(0), height).unwrap();
            let exact = -(beta.cos() * 0.5 * w + beta.sin() * h + 0.5 * w);
            assert!(
                (got - exact).abs() <= 1e-12 * k * 50.0,
                "scale {k}, β {beta}: {got} against {exact}"
            );
            // The hull holds the D: its least is at or below the exact.
            let by_hull = piece_points(&p, PieceOf::Flat(0))
                .into_iter()
                .map(height)
                .fold(f64::INFINITY, f64::min);
            assert!(
                by_hull <= got + 1e-12 * k * 50.0,
                "scale {k}: hull {by_hull}"
            );
        }
    }
}
