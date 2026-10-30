//! Fairness of network sheets held in curvature (G2): the normal curvature
//! along the lines that leave the held edge, measured on the exact surface.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{Condition, Network, Patch, gordon};
use craft3d_geom::refine::interpolate;
use craft3d_geom::{Cylinder, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};

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

/// The guide's fin (docs/guide/09-features.md, "Curvature-continuous
/// seams"): a sheet from a post of radius 20 out to a line at x = 35.
fn fin(kind: &str) -> Patch {
    let post = Surface::Cylinder(Cylinder::new(Frame::WORLD, 20.0).unwrap());
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
    let vs = vec![section(0.0), section(30.0)];
    let edge = match kind {
        "curvature" => Condition::Curvature {
            surface: &post,
            weight: 1.0,
        },
        "tangent" => Condition::Tangent {
            surface: &post,
            weight: 1.0,
        },
        _ => Condition::Free,
    };
    gordon(&Network {
        u: &us,
        v: &vs,
        edges: [edge, Condition::Free, Condition::Free, Condition::Free],
        tolerance: 1e-6,
        angular: 1e-9,
    })
    .unwrap()
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

fn report(label: &str, patch: &Patch) {
    let s = &patch.surface;
    println!(
        "{label}: degrees {:?}, knots v {:?}",
        s.degrees(),
        s.knots_v()
    );
    for u in [0.0, 0.5, 1.0] {
        let p = profile(s, u, 2000);
        let k: Vec<f64> = p.iter().map(|x| x.2).collect();
        let signs = k.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        let slopes: Vec<f64> = p
            .windows(2)
            .map(|w| (w[1].2 - w[0].2) / (w[1].1 - w[0].1))
            .collect();
        let turns = slopes.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        let steepest = slopes.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
        let (lo, hi) = k
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
        println!(
            "  u {u}: kappa at seam {:.5}, at end {:.5}, min {lo:.5}, max {hi:.5}, sign changes {signs}, slope sign changes {turns}, max |dk/ds| {steepest:.5}, length {:.3}",
            k[0],
            k[k.len() - 1],
            p[p.len() - 1].1
        );
        if u == 0.5 {
            let line: Vec<String> = p
                .iter()
                .step_by(25)
                .map(|x| format!("{:.3}:{:.4}", x.0, x.2))
                .collect();
            println!("  profile {}", line.join(" "));
        }
    }
}

#[test]
fn the_guide_fin_profile() {
    report("free", &fin("free"));
    report("tangent", &fin("tangent"));
    report("curvature", &fin("curvature"));
}
