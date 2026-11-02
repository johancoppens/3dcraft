//! A projected trim's edge proven within the tolerance of the true
//! projection everywhere, not only at its samples
//! (docs/design/surfacing.md › *A projected edge, proven*).
//!
//! Oracles: closed-form projections (radial onto a cylinder and a sphere,
//! a line meeting a plane), sampled densely, at 1e-3, 1 and 1e6 with a
//! tolerance of 1e-6 times the scale. The proven bound must be at least
//! the densest sampled gap (it is a bound) and at most half the tolerance
//! (it was accepted). A curve with a bump between the old samples shows
//! what the proof catches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::projection_proof::projection_bound;
use craft3d_geom::trim::{self, Projection};
use craft3d_geom::{Cylinder, NurbsCurve, NurbsSurface, Sphere, Surface};
use craft3d_math::{Frame, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The largest distance, at `n + 1` evenly spaced places, between the
/// edge `S(c(t))` and the closed-form projection of the curve's point.
fn dense_gap(
    form: &NurbsSurface,
    curve: &NurbsCurve,
    uv: &NurbsCurve,
    exact: &dyn Fn(Vec3) -> Vec3,
    n: usize,
) -> f64 {
    let (a, b) = curve.domain();
    let mut worst = 0.0_f64;
    for i in 0..=n {
        let t = a + (b - a) * i as f64 / n as f64;
        let q = uv.eval(t);
        let on = form.eval(q.x, q.y);
        worst = worst.max(on.distance(exact(curve.eval(t))));
    }
    worst
}

fn world() -> Frame {
    Frame::new(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap()
}

/// Proven, accepted, and at least what dense sampling finds.
fn check(
    what: &str,
    form: &NurbsSurface,
    curve: &NurbsCurve,
    how: Projection,
    exact: &dyn Fn(Vec3) -> Vec3,
    tol: f64,
) {
    let made = trim::project_proven(form, curve, how, 0.5 * tol, tol)
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    let dense = dense_gap(form, curve, &made.uv, exact, 20_000);
    println!(
        "{what}: proven {:.3e}, dense {dense:.3e}, sampled {:.3e}, {} samples, {} pieces",
        made.proven, made.sampled, made.samples, made.pieces
    );
    assert!(
        made.proven <= 0.5 * tol,
        "{what}: proven {:e} over half the tolerance {:e}",
        made.proven,
        0.5 * tol
    );
    assert!(
        made.proven >= dense,
        "{what}: proven {:e} below the dense sampled gap {dense:e}",
        made.proven
    );
    assert!(made.sampled <= made.proven, "{what}: {made:?}");
    // The proof is not vacuous: within a few hundred times the true gap
    // or a millionth of the tolerance.
    assert!(
        made.proven <= 1000.0 * dense + 1e-6 * tol,
        "{what}: proven {:e} far above the dense gap {dense:e}",
        made.proven
    );
}

#[test]
fn a_line_projected_onto_a_cylinder() {
    for k in SCALES {
        let r = 10.0 * k;
        let cyl = Surface::Cylinder(Cylinder::new(world(), r).unwrap());
        // A skew line beside the cylinder: its closest points are radial.
        let line = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![
                Vec3::new(1.5 * r, -0.6 * r, -0.4 * r),
                Vec3::new(1.2 * r, 0.7 * r, 0.9 * r),
            ],
        )
        .unwrap();
        let exact = move |p: Vec3| {
            let rho = (p.x * p.x + p.y * p.y).sqrt();
            Vec3::new(r * p.x / rho, r * p.y / rho, p.z)
        };
        let landed: Vec<Vec3> = (0..=8)
            .map(|i| exact(line.eval(f64::from(i) / 8.0)))
            .collect();
        let form = trim::rational_form(&cyl, &landed).unwrap();
        check(
            &format!("cylinder at {k}"),
            &form,
            &line,
            Projection::Normal,
            &exact,
            1e-6 * k,
        );
    }
}

/// A full circle as four rational quadratic quarters, about `c` in the
/// plane z = c.z.
fn circle(c: Vec3, r: f64) -> NurbsCurve {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let pts = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let control: Vec<Vec3> = pts
        .iter()
        .map(|(x, y)| Vec3::new(c.x + r * x, c.y + r * y, c.z))
        .collect();
    let weights = vec![1.0, h, 1.0, h, 1.0, h, 1.0, h, 1.0];
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    NurbsCurve::new(2, knots, control, weights).unwrap()
}

#[test]
fn a_circle_projected_along_a_direction_onto_a_slanted_plane() {
    for k in SCALES {
        let s = 10.0 * k;
        // The plane z = 0.3 x + 0.2 y + s as a bilinear spline.
        let z = |x: f64, y: f64| 0.3 * x + 0.2 * y + s;
        let corner = |x: f64, y: f64| Vec3::new(x, y, z(x, y));
        let plane = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![
                corner(-2.0 * s, -2.0 * s),
                corner(-2.0 * s, 2.0 * s),
                corner(2.0 * s, -2.0 * s),
                corner(2.0 * s, 2.0 * s),
            ],
            vec![1.0; 4],
        )
        .unwrap();
        let d = Vec3::new(0.1, -0.2, -1.0).normalized().unwrap();
        let ring = circle(Vec3::new(0.1 * s, -0.2 * s, 3.0 * s), 0.8 * s);
        let n = Vec3::new(-0.3, -0.2, 1.0);
        let p0 = Vec3::new(0.0, 0.0, s);
        let exact = move |p: Vec3| {
            let lambda = n.dot(p0 - p) / n.dot(d);
            p + d * lambda
        };
        check(
            &format!("slanted plane at {k}"),
            &plane,
            &ring,
            Projection::Along(d),
            &exact,
            1e-6 * k,
        );
    }
}

#[test]
fn a_spline_projected_onto_a_sphere() {
    for k in SCALES {
        let r = 10.0 * k;
        let sphere = Surface::Sphere(Sphere::new(world(), r).unwrap());
        // A cubic outside the sphere, near its equator, bowed up and down.
        let curve = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(1.3 * r, -0.7 * r, 0.1 * r),
                Vec3::new(1.4 * r, -0.2 * r, 0.4 * r),
                Vec3::new(1.5 * r, 0.1 * r, -0.3 * r),
                Vec3::new(1.2 * r, 0.5 * r, 0.2 * r),
                Vec3::new(1.0 * r, 0.8 * r, 0.0),
            ],
        )
        .unwrap();
        let exact = move |p: Vec3| p * (r / p.length());
        let landed: Vec<Vec3> = (0..=8)
            .map(|i| exact(curve.eval(f64::from(i) / 8.0)))
            .collect();
        let form = trim::rational_form(&sphere, &landed).unwrap();
        check(
            &format!("sphere at {k}"),
            &form,
            &curve,
            Projection::Normal,
            &exact,
            1e-6 * k,
        );
    }
}

/// A line along a plane with a narrow bump between the first sample and
/// the first midpoint of 16 samples: the old midpoint test sees nothing.
#[test]
fn a_bump_between_the_samples_is_caught() {
    for k in SCALES {
        let tol = 1e-6 * k;
        let plane = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 2.0 * k, 0.0),
                Vec3::new(3.0 * k, 0.0, 0.0),
                Vec3::new(3.0 * k, 2.0 * k, 0.0),
            ],
            vec![1.0; 4],
        )
        .unwrap();
        // A uniform cubic spline of 256 spans whose x runs linearly (its
        // control points at the Greville abscissae), y = k but for one
        // control point raised by a thousandth of k: its support is
        // [2/256, 6/256], strictly between the sample at 0 and the
        // midpoint at 8/256. Above the plane by z = k (taken off by the
        // projection).
        let spans = 256usize;
        let mut knots = vec![0.0; 4];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend([1.0; 4]);
        let count = spans + 3;
        let control: Vec<Vec3> = (0..count)
            .map(|i| {
                let g = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
                let y = if i == 5 { k + 1e-3 * k } else { k };
                Vec3::new(3.0 * k * g, y, k)
            })
            .collect();
        let bumpy = NurbsCurve::polynomial(3, knots, control).unwrap();
        let exact = |p: Vec3| Vec3::new(p.x, p.y, 0.0);
        let surface = Surface::Nurbs(plane.clone());
        // Before: accepted at 16 samples, nothing found at the midpoints.
        let (old, worst) = trim::project_points(
            &surface,
            &craft3d_geom::Curve3::Nurbs(bumpy.clone()),
            bumpy.domain(),
            Projection::Normal,
            0.5 * tol,
            tol,
        )
        .unwrap();
        assert!(worst <= 0.5 * tol, "{k}: the old test saw {worst:e}");
        let dense = dense_gap(&plane, &bumpy, &old, &exact, 20_000);
        assert!(
            dense > 1e-4 * k,
            "{k}: the old edge misses the bump by {dense:e}"
        );
        // The proof on that old edge: at least the bump, so refused.
        let gap = projection_bound(&plane, &bumpy, &old, Projection::Normal).unwrap();
        assert!(gap.bound >= dense, "{k}: {gap:?} below {dense:e}");
        assert!(gap.bound > 0.5 * tol, "{k}: {gap:?}");
        // Now: refined until proven, and the proof holds densely.
        let made = trim::project_proven(&plane, &bumpy, Projection::Normal, 0.5 * tol, tol)
            .unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(made.samples > 16, "{k}: {made:?}");
        let dense = dense_gap(&plane, &bumpy, &made.uv, &exact, 20_000);
        println!(
            "bump at {k}: old edge off by {:.3e} (proven {:.3e}); now proven {:.3e}, dense {dense:.3e}, {} samples",
            dense_gap(&plane, &bumpy, &old, &exact, 20_000),
            gap.bound,
            made.proven,
            made.samples
        );
        assert!(
            made.proven >= dense && made.proven <= 0.5 * tol,
            "{k}: {made:?} vs {dense:e}"
        );
    }
}

/// A projection that cannot be proven: the curve runs along the
/// cylinder's axis, where every point of a circle is a foot.
#[test]
fn a_curve_along_the_axis_is_not_proven() {
    let r = 10.0;
    let cyl = Surface::Cylinder(Cylinder::new(world(), r).unwrap());
    let landed: Vec<Vec3> = (0..=8).map(|i| Vec3::new(r, 0.0, f64::from(i))).collect();
    let form = trim::rational_form(&cyl, &landed).unwrap();
    // The axis line itself, and a fit of a line beside it at the same
    // parameters: the condition is degenerate along the whole axis.
    let axis = NurbsCurve::polynomial(
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 7.0)],
    )
    .unwrap();
    let beside = NurbsCurve::polynomial(
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![Vec3::new(2.0 * r, 0.0, 1.0), Vec3::new(2.0 * r, 0.0, 7.0)],
    )
    .unwrap();
    let fit = trim::project_proven(&form, &beside, Projection::Normal, 0.5e-6, 1e-6).unwrap();
    let refused = projection_bound(&form, &axis, &fit.uv, Projection::Normal);
    assert!(refused.is_err(), "{refused:?}");
}
