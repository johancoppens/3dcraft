//! TEMPORARY: diagnostics of the curvature hold.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_geom::curvature::second_derivatives;
use craft3d_geom::network::{
    Condition, DEBUG_KEEP, DEBUG_RESIDUAL, Network, arc, continuity, gordon,
};
use craft3d_geom::{NurbsCurve, Sphere, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::FRAC_PI_2;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

#[test]
fn debug_dome() {
    DEBUG_KEEP.store(true, std::sync::atomic::Ordering::Relaxed);
    let r = 1.5;
    let sphere = Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap());
    let top = Frame::new(v(0.0, 0.0, 0.8 * r), v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)).unwrap();
    let us = vec![
        arc(&Frame::WORLD, r, 0.0, FRAC_PI_2).unwrap(),
        arc(&top, 0.7 * r, 0.0, FRAC_PI_2).unwrap(),
    ];
    let vs: Vec<NurbsCurve> = [0.0, 0.5, 1.0]
        .iter()
        .map(|f: &f64| {
            let a = FRAC_PI_2 * f;
            let d = v(a.cos(), a.sin(), 0.0);
            NurbsCurve::polynomial(
                3,
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                vec![
                    d * r,
                    d * r + v(0.0, 0.0, 0.3 * r),
                    d * (0.85 * r) + v(0.0, 0.0, 0.65 * r),
                    d * (0.7 * r) + v(0.0, 0.0, 0.8 * r),
                ],
            )
            .unwrap()
        })
        .collect();
    for kind in [false, true] {
        let c = if kind {
            Condition::Curvature {
                surface: &sphere,
                weight: 1.0,
            }
        } else {
            Condition::Tangent {
                surface: &sphere,
                weight: 1.0,
            }
        };
        let patch = gordon(&Network {
            u: &us,
            v: &vs,
            edges: [c, Condition::Free, Condition::Free, Condition::Free],
            tolerance: 1e-6,
            angular: 1e-9,
        })
        .unwrap();
        let res = f64::from_bits(DEBUG_RESIDUAL.load(std::sync::atomic::Ordering::Relaxed));
        println!(
            "KIND g2={kind} residual-code {res:e} degrees {:?} counts {:?} ku {:?} kv {:?}",
            patch.surface.degrees(),
            patch.surface.counts(),
            patch.surface.knots_u(),
            patch.surface.knots_v()
        );
        let w = Surface::Nurbs(patch.surface.clone());
        for k in 0..=16 {
            let u = f64::from(k) / 16.0;
            let (p, du, dv) = w.derivatives(u, 0.0);
            let (_, _, dvv) = second_derivatives(&w, u, 0.0);
            let n = du.cross(dv).normalized().unwrap();
            let kk = n.dot(dvv) / dv.dot(dv) * n.dot(p.normalized().unwrap()).signum();
            println!(
                "  u {u:.4} kappa+1/r {:e} sine {:e}",
                kk + 1.0 / r,
                n.cross(p.normalized().unwrap()).length()
            );
        }
        let c = continuity(&w, &sphere, &patch.boundary[0].clone().unwrap(), 1e-6, 1e-9).unwrap();
        println!("  continuity {c:?}");
    }
}
