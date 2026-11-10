//! Fairness of network sheets held in curvature (G2): the normal curvature
//! along the lines that leave the held edge, measured on the exact surface.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{Condition, Network, Patch, arc, gordon};
use craft3d_geom::refine::interpolate;
use craft3d_geom::{Cylinder, NurbsCurve, NurbsSurface, Plane, Sphere, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use std::f64::consts::FRAC_PI_2;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(a: Vec3, b: Vec3) -> NurbsCurve {
    NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap()
}

/// The cubic through four points at 0, 1/3, 2/3, 1 (as `sketch.spline
/// {through}` fits four points).
fn through(points: [Vec3; 4]) -> NurbsCurve {
    let values: Vec<Vec<f64>> = points.iter().map(|p| vec![p.x, p.y, p.z]).collect();
    let (knots, ctrl) = interpolate(&[0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0], 3, &values).unwrap();
    let ctrl = ctrl.iter().map(|c| v(c[0], c[1], c[2])).collect();
    NurbsCurve::polynomial(3, knots, ctrl).unwrap()
}

/// A case: its curves and the face its first u curve is held to.
struct Case {
    name: &'static str,
    us: Vec<NurbsCurve>,
    vs: Vec<NurbsCurve>,
    face: Surface,
}

fn patch(case: &Case, kind: &str) -> Patch {
    let edge = match kind {
        "curvature" => Condition::Curvature {
            surface: &case.face,
            weight: 1.0,
        },
        "tangent" => Condition::Tangent {
            surface: &case.face,
            weight: 1.0,
        },
        _ => Condition::Free,
    };
    let size = case
        .us
        .iter()
        .chain(&case.vs)
        .flat_map(|c| c.control_points().iter().map(|p| p.length()))
        .fold(0.0_f64, f64::max);
    gordon(&Network {
        u: &case.us,
        v: &case.vs,
        edges: [edge, Condition::Free, Condition::Free, Condition::Free],
        tolerance: 1e-6 * size,
        angular: 1e-9,
    })
    .unwrap()
}

fn bezier(points: [Vec3; 4]) -> NurbsCurve {
    NurbsCurve::polynomial(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        points.to_vec(),
    )
    .unwrap()
}

/// The guide's fin (docs/guide/features.md, "Curvature-continuous
/// seams"): a sheet from a post of radius 20 out to a line at x = 35.
fn fin() -> Case {
    let us = vec![
        line(v(20.0, 0.0, 0.0), v(20.0, 0.0, 30.0)),
        line(v(35.0, 25.0, 0.0), v(35.0, 25.0, 30.0)),
    ];
    let section = |z: f64| {
        through([
            v(20.0, 0.0, z),
            v(23.0, 6.5, z),
            v(27.0, 13.0, z),
            v(35.0, 25.0, z),
        ])
    };
    Case {
        name: "fin (cylinder)",
        us,
        vs: vec![section(0.0), section(30.0)],
        face: Surface::Cylinder(Cylinder::new(Frame::WORLD, 20.0).unwrap()),
    }
}

/// A sheet rising from the plane z = 0 along the x axis, its sections
/// leaving at 30° and curving up.
fn ramp() -> Case {
    let section = |x: f64, lift: f64| {
        bezier([
            v(x, 0.0, 0.0),
            v(x, 4.0, 2.3),
            v(x, 8.0, 3.0 + lift),
            v(x, 12.0, 6.0 + lift),
        ])
    };
    Case {
        name: "ramp (plane)",
        us: vec![
            line(v(0.0, 0.0, 0.0), v(20.0, 0.0, 0.0)),
            line(v(0.0, 12.0, 6.0), v(20.0, 12.0, 7.0)),
        ],
        vs: vec![section(0.0, 0.0), section(10.0, 0.5), section(20.0, 1.0)],
        face: Surface::Plane(Plane::new(Frame::WORLD)),
    }
}

/// A dome over a quarter of the equator of a sphere of radius 1.5 (as
/// `network_g2.rs`): it leaves the equator upwards, ends on a smaller arc.
fn dome_curves(r: f64) -> (Vec<NurbsCurve>, Vec<NurbsCurve>) {
    let top = Frame::new(v(0.0, 0.0, 0.8 * r), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&top, 0.7 * r, 0.0, FRAC_PI_2).unwrap(),
    ];
    let vs = [0.0, 0.5, 1.0]
        .iter()
        .map(|f: &f64| {
            let a = FRAC_PI_2 * f;
            let d = v(a.pcos(), a.psin(), 0.0);
            bezier([
                d * r,
                d * r + v(0.0, 0.0, 0.3 * r),
                d * (0.85 * r) + v(0.0, 0.0, 0.65 * r),
                d * (0.7 * r) + v(0.0, 0.0, 0.8 * r),
            ])
        })
        .collect();
    (us, vs)
}

fn dome() -> Case {
    let (us, vs) = dome_curves(1.5);
    Case {
        name: "dome (sphere)",
        us,
        vs,
        face: Surface::Sphere(Sphere::new(Frame::WORLD, 1.5).unwrap()),
    }
}

/// The dome held to a network sheet: the sphere's lower octant through
/// its meridians (a spline face along its edge's parameter line).
fn dome_on_spline() -> Case {
    let r = 1.5;
    let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
        .iter()
        .map(|f: &f64| {
            let a = FRAC_PI_2 * f;
            let q = v(a.pcos(), a.psin(), 0.0);
            let frame = Frame::new(v(0.0, 0.0, 0.0), v(q.y, -q.x, 0.0), q).unwrap();
            arc(&frame, r, 0.0, -FRAC_PI_2).unwrap()
        })
        .collect();
    let middle = vs[0].eval(0.5);
    let parallel = Frame::new(v(0.0, 0.0, middle.z), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let pole = v(0.0, 0.0, -r);
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&parallel, middle.x, 0.0, FRAC_PI_2).unwrap(),
        line(pole, pole),
    ];
    let octant = gordon(&Network {
        u: &us,
        v: &vs,
        edges: [Condition::Free; 4],
        tolerance: 1e-6 * r,
        angular: 1e-9,
    })
    .unwrap();
    let (us, vs) = dome_curves(r);
    Case {
        name: "dome (spline face)",
        us,
        vs,
        face: Surface::Nurbs(octant.surface),
    }
}

/// The normal curvature of `s` along the line u = `u` (in its direction)
/// and the arc length, at `count + 1` places from v = 0 to 1.
fn profile(s: &NurbsSurface, u: f64, count: usize) -> Vec<(f64, f64, f64)> {
    let wrapped = Surface::Nurbs(s.clone());
    let mut out = Vec::new();
    let mut length = 0.0;
    let mut last: Option<Vec3> = None;
    for k in 0..=count {
        let t = k as f64 / count as f64;
        let (p, du, dv) = wrapped.derivatives(u, t);
        let (_, _, dvv) = second_derivatives(&wrapped, u, t);
        let n = du.cross(dv).normalized().unwrap();
        if let Some(q) = last {
            length += p.distance(q);
        }
        last = Some(p);
        out.push((t, length, n.dot(dvv) / dv.dot(dv)));
    }
    out
}

/// What a profile shows: the curvature at the seam, its least and
/// largest, the changes of sign of the curvature and of its slope (a
/// slope below a millionth of the steepest is taken as none), the
/// steepest slope.
#[derive(Debug, Clone, Copy)]
struct Shape {
    seam: f64,
    low: f64,
    high: f64,
    signs: usize,
    turns: usize,
    steepest: f64,
}

fn shape(p: &[(f64, f64, f64)]) -> Shape {
    let k: Vec<f64> = p.iter().map(|x| x.2).collect();
    let signs = k.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
    let slopes: Vec<f64> = p
        .windows(2)
        .map(|w| (w[1].2 - w[0].2) / (w[1].1 - w[0].1))
        .collect();
    let steepest = slopes.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
    let real: Vec<f64> = slopes
        .iter()
        .copied()
        .filter(|x| x.abs() > 1e-6 * steepest)
        .collect();
    let turns = real.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
    let (low, high) = k
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
    Shape {
        seam: k[0],
        low,
        high,
        signs,
        turns,
        steepest,
    }
}

const LINES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn shapes(patch: &Patch) -> Vec<Shape> {
    LINES
        .iter()
        .map(|u| shape(&profile(&patch.surface, *u, 2000)))
        .collect()
}

/// Along the lines that leave a curvature edge at the crossing curves
/// (u = 0, ½, 1), the normal curvature runs from the face's to the
/// sheet's own with one turn at most (a single extremum), changes sign at
/// most once, stays within the band between the face's curvature and the
/// tangent-held sheet's, widened by at most 0.75 of the change the hold
/// makes at the seam (the least-variation change must swing past the
/// sheet's own by about that, see `network::fair`), and is at most three
/// times as steep as the tangent-held sheet. On every line (between the
/// crossings too) it is at most three times as steep as the steeper of
/// the free and the tangent-held sheets anywhere. Before the change was
/// spread (one control point per condition, squeezed into the first knot
/// spans) the guide's fin had four changes of sign, three turns, a swing
/// of 3.1 times the change at the seam and 13 times the tangent sheet's
/// slope.
#[test]
fn curvature_held_sheets_are_fair_across_their_seam() {
    for case in [fin(), ramp(), dome(), dome_on_spline()] {
        let free = shapes(&patch(&case, "free"));
        let tangent = shapes(&patch(&case, "tangent"));
        let held = shapes(&patch(&case, "curvature"));
        let steepest_own = free
            .iter()
            .chain(&tangent)
            .fold(0.0_f64, |m, s| m.max(s.steepest));
        for (k, u) in LINES.iter().enumerate() {
            let (g1, g2) = (tangent[k], held[k]);
            let what = format!("{} at u = {u}: held {g2:?}, tangent {g1:?}", case.name);
            assert!(g2.steepest <= 3.0 * steepest_own, "{what}");
            if ![0.0, 0.5, 1.0].contains(u) {
                continue;
            }
            assert!(g2.turns <= 1, "{what}");
            assert!(g2.signs <= 1, "{what}");
            let change = (g2.seam - g1.seam).abs();
            let (low, high) = (g1.low.min(g2.seam), g1.high.max(g2.seam));
            let swing = (low - g2.low).max(g2.high - high).max(0.0);
            assert!(swing <= 0.75 * change + 1e-9, "{what}: swing {swing}");
            assert!(g2.steepest <= 3.0 * g1.steepest, "{what}");
        }
    }
}

#[test]
#[ignore = "prints the profiles: run with --ignored --nocapture"]
fn profiles_across_held_edges() {
    for case in [fin(), ramp(), dome(), dome_on_spline()] {
        for kind in ["free", "tangent", "curvature"] {
            let p = patch(&case, kind);
            println!(
                "{} {kind}: degrees {:?}, knots v {:?}",
                case.name,
                p.surface.degrees(),
                p.surface.knots_v()
            );
            for (u, s) in LINES.iter().zip(shapes(&p)) {
                println!(
                    "  u {u}: seam {:.5}, low {:.5}, high {:.5}, signs {}, turns {}, steepest {:.5}",
                    s.seam, s.low, s.high, s.signs, s.turns, s.steepest
                );
            }
            for u in [0.5, 0.75] {
                let mid = profile(&p.surface, u, 2000);
                let line: Vec<String> = mid
                    .iter()
                    .step_by(if u == 0.5 { 50 } else { 10 })
                    .map(|x| format!("{:.3}:{:.4}", x.0, x.2))
                    .collect();
                println!("  profile at u {u}: {}", line.join(" "));
            }
        }
    }
}
