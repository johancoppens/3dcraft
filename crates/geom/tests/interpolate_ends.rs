//! Cubic interpolation with given end derivatives
//! (`refine::interpolate_with_end_derivatives`): a cubic polynomial is in
//! the spline's space, so it is reproduced exactly from its values at any
//! increasing parameters and its derivatives at the ends; the end tangents
//! are the given ones; bad input is an error, not a panic.

use craft3d_geom::NurbsCurve;
use craft3d_geom::refine::interpolate_with_end_derivatives;
use craft3d_math::Vec3;

fn cubic(u: f64, k: f64) -> Vec3 {
    Vec3::new(u * u * u - 2.0 * u, 3.0 * u * u - u, 1.0 + 0.5 * u) * k
}

fn cubic_rate(u: f64, k: f64) -> Vec3 {
    Vec3::new(3.0 * u * u - 2.0, 6.0 * u - 1.0, 0.5) * k
}

fn row(p: Vec3) -> Vec<f64> {
    vec![p.x, p.y, p.z]
}

fn curve(knots: Vec<f64>, ctrl: &[Vec<f64>]) -> NurbsCurve {
    let ctrl = ctrl.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect();
    NurbsCurve::polynomial(3, knots, ctrl).unwrap()
}

#[test]
fn a_cubic_is_reproduced_from_its_values_and_end_derivatives() {
    for k in [1e-3, 1.0, 1e6] {
        for params in [
            vec![0.0, 1.0],
            vec![0.0, 0.3, 1.0],
            vec![0.0, 1e-4, 2e-4, 0.5, 0.75, 1.0],
            vec![-2.0, -1.9, -0.5, 0.0, 0.1, 1.5, 3.0],
        ] {
            let values: Vec<Vec<f64>> = params.iter().map(|&u| row(cubic(u, k))).collect();
            let (u0, un) = (params[0], *params.last().unwrap());
            let (knots, ctrl) = interpolate_with_end_derivatives(
                &params,
                &values,
                &row(cubic_rate(u0, k)),
                &row(cubic_rate(un, k)),
            )
            .unwrap();
            assert_eq!(ctrl.len(), params.len() + 2);
            assert_eq!(knots.len(), ctrl.len() + 4);
            let c = curve(knots, &ctrl);
            for i in 0..=64 {
                let u = u0 + (un - u0) * i as f64 / 64.0;
                let d = c.eval(u).distance(cubic(u, k));
                assert!(d <= 1e-12 * k * 64.0, "{k} {params:?}: {d} at {u}");
            }
            for u in [u0, un] {
                let (_, rate) = c.derivatives(u);
                let want = cubic_rate(u, k);
                assert!(
                    rate.distance(want) <= 1e-9 * want.length(),
                    "{k} {params:?}: {rate:?} at {u}"
                );
            }
        }
    }
}

#[test]
fn bad_input_is_refused() {
    let p = vec![0.0, 0.5, 1.0];
    let v = vec![vec![0.0; 3]; 3];
    let d = vec![1.0, 0.0, 0.0];
    assert!(interpolate_with_end_derivatives(&p, &v, &d, &d).is_ok());
    assert!(interpolate_with_end_derivatives(&[0.0], &v[..1], &d, &d).is_err());
    assert!(interpolate_with_end_derivatives(&[0.0, 0.0, 1.0], &v, &d, &d).is_err());
    assert!(interpolate_with_end_derivatives(&[0.0, f64::NAN, 1.0], &v, &d, &d).is_err());
    assert!(interpolate_with_end_derivatives(&p, &v[..2], &d, &d).is_err());
    assert!(interpolate_with_end_derivatives(&p, &v, &d[..2], &d).is_err());
    assert!(interpolate_with_end_derivatives(&p, &v, &d, &[f64::INFINITY, 0.0, 0.0]).is_err());
    let ragged = vec![vec![0.0; 3], vec![0.0; 2], vec![0.0; 3]];
    assert!(interpolate_with_end_derivatives(&p, &ragged, &d, &d).is_err());
}
