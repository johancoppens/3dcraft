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
//! - the lowest point of the part of a flat or a bend on one side of a
//!   plane (the proof of material past a bend line, *Past the bend
//!   line*): a D cut across its arch, exact where the plane is square to
//!   the flat and the half widened by the plane's lean otherwise; a bend
//!   cut along its length;
//! - a die cut into triangles: as many as its points less two, each
//!   counter-clockwise, their areas adding up to the die's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::sequence::{
    PieceOf, final_worlds, piece_lowest, piece_lowest_within, piece_points, simple, triangles,
};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, FlangeSpec, LengthFrom, On, Part, Spec, build,
};
use craft3d_math::Portable;
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
            bend_relief: None,
            corner: Closing::OPEN,
            corners: Vec::new(),
            cutouts: Vec::new(),
            jogs: Vec::new(),
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
        turn * cross >= -slack * dx.phypot(dy)
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
    let (c, s) = ((PI / 6.0).pcos(), (PI / 6.0).psin());
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
                alpha.pcos() * (q.z - zc) - alpha.psin() * u
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
                -(beta.pcos() * q.x + beta.psin() * q.y) + 0.25 * q.z
            };
            let got = piece_lowest(&p, PieceOf::Flat(0), height).unwrap();
            let exact = -(beta.pcos() * 0.5 * w + beta.psin() * h + 0.5 * w);
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

/// The convex hull of `points` (counter-clockwise), by the monotone chain.
fn convex_hull(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p = points.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f64; 2]> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let list: Vec<[f64; 2]> = if pass == 0 {
            p.clone()
        } else {
            p.iter().rev().copied().collect()
        };
        for q in list {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0
            {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    hull
}

fn arc_segment(
    center: [f64; 2],
    radius: f64,
    from: f64,
    to: f64,
    reversed: bool,
) -> craft3d_kernel::profile::Segment {
    let at = |a: f64| [center[0] + radius * a.pcos(), center[1] + radius * a.psin()];
    // Counter-clockwise from `from` to `to`; a reversed one runs back.
    let (start, end) = if reversed {
        (at(to), at(from))
    } else {
        (at(from), at(to))
    };
    craft3d_kernel::profile::Segment {
        source: 1,
        piece: 0,
        reversed,
        start,
        end,
        shape: craft3d_kernel::profile::Shape::Arc { center, radius },
    }
}

/// The hull holds every arc of an outline whichever way it runs and
/// bulges (review 2026-10-29): a counter-clockwise arc, a reversed
/// (clockwise) one, one bulging inwards (its sweep the short way round
/// the far side), and a whole circle; 256 points of each lie in the
/// convex hull of `outline_hull`'s points, at three scales.
#[test]
fn the_hull_holds_arcs_either_way_round_and_whole_circles() {
    use craft3d_kernel::sheetmetal::sequence::outline_hull;
    for k in SCALES {
        let cases = [
            (arc_segment([0.0, 0.0], 10.0 * k, 0.3, 2.6, false), 0.3, 2.6),
            (arc_segment([0.0, 0.0], 10.0 * k, 0.3, 2.6, true), 0.3, 2.6),
            // Inward: the arc about a centre outside the region, its
            // reversed run the long way (here 300 degrees).
            (
                arc_segment([5.0 * k, 5.0 * k], 4.0 * k, -2.0, 3.236, true),
                -2.0,
                3.236,
            ),
        ];
        for (seg, from, to) in &cases {
            let hull = convex_hull(&outline_hull(std::slice::from_ref(seg)));
            let craft3d_kernel::profile::Shape::Arc { center, radius } = seg.shape else {
                panic!("an arc")
            };
            for j in 0..=256 {
                let a = from + (to - from) * j as f64 / 256.0;
                let p = [center[0] + radius * a.pcos(), center[1] + radius * a.psin()];
                assert!(
                    in_convex(&hull, p, 1e-9 * k),
                    "scale {k}: {p:?} of {seg:?} outside the hull {hull:?}"
                );
            }
        }
        let (c, r) = ([3.0 * k, -2.0 * k], 7.0 * k);
        let circle = craft3d_kernel::profile::Segment {
            source: 2,
            piece: 0,
            reversed: true,
            start: [c[0] + r, c[1]],
            end: [c[0] + r, c[1]],
            shape: craft3d_kernel::profile::Shape::Circle {
                center: c,
                radius: r,
            },
        };
        let hull = convex_hull(&outline_hull(&[circle]));
        for j in 0..256 {
            let a = std::f64::consts::TAU * j as f64 / 256.0;
            let p = [c[0] + r * a.pcos(), c[1] + r * a.psin()];
            assert!(in_convex(&hull, p, 1e-9 * k), "scale {k}: {p:?} outside");
        }
    }
}

/// The D of `an_arched_flats_lowest_point_is_its_arcs_least` (width `w`,
/// height `h` to the arch's springing, the arch a half circle).
fn d_shape(k: f64) -> (Vec<SketchCurve>, f64, f64) {
    let (w, h) = (20.0 * k, 10.0 * k);
    let line = |id, start, end| SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    };
    let curves = vec![
        line(1, [0.0, 0.0], [w, 0.0]),
        line(2, [w, 0.0], [w, h]),
        SketchCurve {
            id: 3,
            curve: Curve2::Arc {
                center: [0.5 * w, h],
                start: [w, h],
                end: [0.0, h],
            },
        },
        line(4, [0.0, h], [0.0, 0.0]),
    ];
    (curves, w, h)
}

/// The least of `−(cos β x + sin β y)` over the D cut to `x ≥ c`, in
/// closed form: at the cut rectangle's corners, the arch's end on the
/// right, where the cut meets the arch, and the arch's point facing
/// `−(cos β, sin β)` when it lies in the half kept.
fn d_least(w: f64, h: f64, c: f64, beta: f64) -> f64 {
    let f = |x: f64, y: f64| -(beta.pcos() * x + beta.psin() * y);
    let r = 0.5 * w;
    let meet = ((c - r) / r).clamp(-1.0, 1.0).pacos();
    let mut least = f(c, 0.0).min(f(w, 0.0)).min(f(w, h));
    least = least.min(f(r + r * meet.pcos(), h + r * meet.psin()));
    // On the arch (angles 0 to `meet` from its centre) the function is
    // least at the angle β.
    let b = beta.rem_euclid(2.0 * PI);
    if b <= meet {
        least = least.min(f(r + r * b.pcos(), h + r * b.psin()));
    }
    least
}

#[test]
fn a_flats_half_past_a_plane_has_its_own_least() {
    for k in SCALES {
        let (curves, w, h) = d_shape(k);
        let t = 1.0 * k;
        let mut f = flange(1, 1.0 * k);
        f.length = 5.0 * k;
        let p = part(one(&curves, k), t, vec![f], k);
        let back = final_worlds(&p)[0].inverse().unwrap();
        for (beta, c) in [(0.4_f64, 0.7), (1.1, 0.3), (2.0, 0.55), (2.9, 0.8)] {
            let c = c * w;
            let height = |x: Vec3| {
                let q = back.point(x);
                -(beta.pcos() * q.x + beta.psin() * q.y) + 0.25 * q.z
            };
            // Square to the flat: exact.
            let side = |x: Vec3| back.point(x).x - c;
            let got = piece_lowest_within(&p, PieceOf::Flat(0), height, side).unwrap();
            let exact = d_least(w, h, c, beta);
            assert!(
                (got - exact).abs() <= 1e-12 * k * 50.0,
                "scale {k}, β {beta}, c {c}: {got} against {exact}"
            );
            // The other half: the least of the whole D is the less of the
            // two halves'.
            let other = piece_lowest_within(&p, PieceOf::Flat(0), height, |x| -side(x)).unwrap();
            let whole = piece_lowest(&p, PieceOf::Flat(0), height).unwrap();
            assert!(
                (other.min(got) - whole).abs() <= 1e-12 * k * 50.0,
                "scale {k}, β {beta}: halves {other}, {got}, whole {whole}"
            );
            // Leaning across the thickness by `λ`: the half kept widened
            // to every point where `x − c + λ z ≥ 0` at some level, so
            // `x ≥ c − λ T`.
            let lambda = 0.5;
            let leaning = |x: Vec3| {
                let q = back.point(x);
                q.x - c + lambda * q.z
            };
            let got = piece_lowest_within(&p, PieceOf::Flat(0), height, leaning).unwrap();
            let exact = d_least(w, h, c - lambda * t, beta);
            assert!(
                (got - exact).abs() <= 1e-12 * k * 50.0,
                "scale {k}, β {beta}, leaning: {got} against {exact}"
            );
            // Nothing past a plane beyond the D.
            let none =
                piece_lowest_within(&p, PieceOf::Flat(0), height, |x| back.point(x).x - 2.0 * w)
                    .unwrap();
            assert_eq!(none, f64::INFINITY, "scale {k}");
        }
    }
}

#[test]
fn a_bends_half_past_a_plane_square_to_its_axis() {
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
        let len = (b.end[0] - b.start[0]).phypot(b.end[1] - b.start[1]);
        let d = [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len];
        let along = |q: Vec3| (q.x - b.start[0]) * d[0] + (q.y - b.start[1]) * d[1];
        for (alpha, cut) in [(0.3_f64, 0.25), (0.7, 0.5), (1.2, 0.9)] {
            let a0 = cut * len;
            // The skin touching a plane at the angle α, rising along the
            // axis by a tenth: least `−(R + T)` at the cut's end.
            let height = |x: Vec3| {
                let q = back.point(x);
                let u = (q.x - b.start[0]) * b.across[0] + (q.y - b.start[1]) * b.across[1];
                alpha.pcos() * (q.z - zc) - alpha.psin() * u + 0.1 * along(q)
            };
            let side = |x: Vec3| along(back.point(x)) - a0;
            let got = piece_lowest_within(&p, PieceOf::Bend(0), height, side).unwrap();
            let exact = -(b.radius + t) + 0.1 * a0;
            assert!(
                (got - exact).abs() <= 1e-12 * k * 50.0,
                "scale {k}, α {alpha}: {got} against {exact}"
            );
            // The half before the cut: least at the bend's start.
            let other = piece_lowest_within(&p, PieceOf::Bend(0), height, |x| -side(x)).unwrap();
            assert!(
                (other + (b.radius + t)).abs() <= 1e-12 * k * 50.0,
                "scale {k}, α {alpha}: {other}"
            );
        }
    }
}

#[test]
fn a_die_cut_into_triangles_adds_up_to_its_area() {
    let area = |p: &[[f64; 2]]| -> f64 {
        0.5 * (0..p.len())
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
    };
    for k in SCALES {
        let v_die: Vec<[f64; 2]> = [
            [-20.0, -20.0],
            [20.0, -20.0],
            [20.0, 0.0],
            [8.0, 0.0],
            [0.0, -8.0],
            [-8.0, 0.0],
            [-20.0, 0.0],
        ]
        .iter()
        .map(|p| [p[0] * k, p[1] * k])
        .collect();
        // Clockwise too.
        let reversed: Vec<[f64; 2]> = v_die.iter().rev().copied().collect();
        for die in [&v_die, &reversed] {
            let cut = triangles(die).unwrap();
            assert_eq!(cut.len(), die.len() - 2, "scale {k}");
            let mut sum = 0.0;
            for tri in &cut {
                let a = area(tri);
                assert!(a > 0.0, "scale {k}: {tri:?}");
                sum += a;
            }
            let whole = area(die).abs();
            assert!(
                (sum - whole).abs() <= 1e-12 * whole,
                "scale {k}: {sum} against {whole}"
            );
        }
        // A bow tie crosses itself: no area to cut, refused.
        let bow = [[0.0, 0.0], [k, k], [k, 0.0], [0.0, k]];
        assert!(triangles(&bow).is_none(), "scale {k}");
        // A polygon whose third side dips across its first (as a round
        // shoulder's tangent apex can cross a V's flank) has an area and
        // ears to clip, but is no region: refused, never cut (review of
        // 4116ed3e).
        let crossed: Vec<[f64; 2]> = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [6.0, -1.0],
            [0.0, 10.0],
        ]
        .iter()
        .map(|p| [p[0] * k, p[1] * k])
        .collect();
        assert!(!simple(&crossed), "scale {k}");
        assert!(triangles(&crossed).is_none(), "scale {k}");
        // Touching itself at a corner is not simple either; the V die is.
        let touching: Vec<[f64; 2]> = [[0.0, 0.0], [10.0, 0.0], [5.0, 0.0], [5.0, 5.0]]
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect();
        assert!(!simple(&touching), "scale {k}");
        assert!(simple(&v_die), "scale {k}");
    }
}

/// A tool's outline with arcs: the arc through three points, its top,
/// the polygon that holds it, its mirror image and whether it is its own
/// (sheetmetal.md › *Bend sequence* › *Arcs in the tools*).
#[test]
fn a_tools_arcs_are_circles_through_their_points() {
    use craft3d_kernel::sheetmetal::sequence::{Tool, circle_through};
    for k in SCALES {
        // A punch with a round tip of radius 2 about the origin and
        // straight flanks 50 high.
        let r = 2.0 * k;
        let punch = Tool {
            corners: vec![[-r, 50.0 * k], [-r, 0.0], [r, 0.0], [r, 50.0 * k]],
            through: vec![None, Some([0.0, -r]), None, None],
        };
        let (c, radius, ccw) = circle_through([-r, 0.0], [0.0, -r], [r, 0.0]).unwrap();
        assert!(c[0].abs() <= 1e-15 * k && c[1].abs() <= 1e-15 * k, "{c:?}");
        assert!((radius - r).abs() <= 1e-15 * k, "{radius}");
        assert!(ccw);
        // Its curves: the tip an arc about the origin, counter-clockwise
        // from its left end to its right (as the outline runs).
        let curves = punch.curves().unwrap();
        assert!(matches!(
            curves[1].curve,
            Curve2::Arc { start, end, .. } if start == [-r, 0.0] && end == [r, 0.0]
        ));
        assert!(punch.symmetric(), "scale {k}");
        assert!(punch.mirrored().symmetric());
        // Its top is its flanks', the tip below them.
        assert_eq!(punch.top(), 50.0 * k);
        // The polygon holding it: the tip by its tangents' meeting
        // points, (−r, −r) and (r, −r).
        let held = punch.enclosing().unwrap();
        for corner in [[-r, -r], [r, -r]] {
            assert!(
                held.iter().any(|p| (p[0] - corner[0]).abs() <= 1e-12 * k
                    && (p[1] - corner[1]).abs() <= 1e-12 * k),
                "scale {k}: {held:?}"
            );
        }
        let cut = triangles(&held).unwrap();
        let area: f64 = cut
            .iter()
            .map(|t| {
                0.5 * ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1])
                    - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0]))
            })
            .sum();
        // The flanks' rectangle and the square under it.
        let exact = 2.0 * r * 50.0 * k + 2.0 * r * r;
        assert!(
            (area - exact).abs() <= 1e-9 * exact,
            "scale {k}: {area} against {exact}"
        );

        // A die with a shoulder bulging above its top: its top is the
        // circle's, `c + r` over the arc.
        let die = Tool {
            corners: vec![
                [-10.0 * k, -10.0 * k],
                [10.0 * k, -10.0 * k],
                [10.0 * k, 0.0],
                [-10.0 * k, 0.0],
            ],
            through: vec![None, None, Some([0.0, 1.0 * k]), None],
        };
        let (c, radius, _) = circle_through([10.0 * k, 0.0], [0.0, k], [-10.0 * k, 0.0]).unwrap();
        assert!(
            (die.top() - (c[1] + radius)).abs() <= 1e-12 * k,
            "scale {k}"
        );
        assert!((die.top() - k).abs() <= 1e-12 * k, "scale {k}");
        // An arc that bulges into the region is held by its chord: a die
        // with a round notch keeps its straight top.
        let notched = Tool {
            corners: vec![
                [-10.0 * k, -10.0 * k],
                [10.0 * k, -10.0 * k],
                [10.0 * k, 0.0],
                [-10.0 * k, 0.0],
            ],
            through: vec![None, None, Some([0.0, -k]), None],
        };
        assert_eq!(notched.enclosing().unwrap().len(), 4, "scale {k}");
        assert_eq!(notched.top(), 0.0);
        // An asymmetric arc: not its own mirror image; mirrored twice it
        // is itself.
        let leaning = Tool {
            corners: vec![[-r, 50.0 * k], [-r, 0.0], [r, 0.0], [r, 50.0 * k]],
            through: vec![None, Some([0.5 * r, -0.8 * r]), None, None],
        };
        assert!(!leaning.symmetric(), "scale {k}");
        assert_eq!(leaning.mirrored().mirrored(), leaning);
        // Through points in a line with the side's ends: no arc.
        let flat = Tool {
            corners: vec![[-r, 50.0 * k], [-r, 0.0], [r, 0.0], [r, 50.0 * k]],
            through: vec![None, Some([0.0, 0.0]), None, None],
        };
        assert!(flat.curves().unwrap_err().to_string().contains("in a line"));
        assert!(circle_through([0.0, 0.0], [k, 0.0], [2.0 * k, 0.0]).is_none());
    }
}
