//! Helpers shared by the intersection tests.
#![allow(dead_code)]

use craft3d_geom::intersect::{Meeting, MeetingCurve, SurfaceMeeting};
use craft3d_geom::{Cone, Curve3, Cylinder, Plane, Sphere, Surface, Torus};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

pub fn tol(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale, 1e-9).unwrap()
}

/// A frame with no axis along a world axis.
pub fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

pub fn cube(center: Vec3, half: f64) -> Bbox3 {
    Bbox3::from_points([
        center - Vec3::new(half, half, half),
        center + Vec3::new(half, half, half),
    ])
}

pub fn plane(f: Frame) -> Surface {
    Surface::Plane(Plane::new(f))
}
pub fn cylinder(f: Frame, r: f64) -> Surface {
    Surface::Cylinder(Cylinder::new(f, r).unwrap())
}
pub fn cone(f: Frame, r: f64, half_angle: f64) -> Surface {
    Surface::Cone(Cone::new(f, r, half_angle).unwrap())
}
pub fn sphere(f: Frame, r: f64) -> Surface {
    Surface::Sphere(Sphere::new(f, r).unwrap())
}
pub fn torus(f: Frame, major: f64, minor: f64) -> Surface {
    Surface::Torus(Torus::new(f, major, minor).unwrap())
}

/// Distance from `p` to a surface, through its own `project`.
pub fn distance(surface: &Surface, p: Vec3) -> f64 {
    let (u, v) = surface.project(p).unwrap();
    surface.eval(u, v).distance(p)
}

pub fn normal_at(surface: &Surface, p: Vec3) -> Option<Vec3> {
    let (u, v) = surface.project(p).unwrap();
    surface.normal(u, v)
}

/// `n + 1` points of a meeting curve, the ends included.
pub fn samples(curve: &MeetingCurve, n: usize) -> Vec<Vec3> {
    let (t0, t1) = curve.range;
    assert!(t0 < t1 && t0.is_finite() && t1.is_finite(), "{curve:?}");
    (0..=n)
        .map(|k| curve.curve.eval(t0 + (t1 - t0) * k as f64 / n as f64))
        .collect()
}

pub fn start(curve: &MeetingCurve) -> Vec3 {
    curve.curve.eval(curve.range.0)
}
pub fn end(curve: &MeetingCurve) -> Vec3 {
    curve.curve.eval(curve.range.1)
}

pub fn meeting(found: SurfaceMeeting) -> Meeting {
    match found {
        SurfaceMeeting::Meets(m) => m,
        other => panic!("expected curves or points, got {other:?}"),
    }
}

/// Every curve lies on both surfaces within its stated bound (and a little
/// for rounding), inside the box, closes if it says so, and its tangent
/// flag agrees with the angle between the normals.
pub fn check_meeting(m: &Meeting, a: &Surface, b: &Surface, within: &Bbox3, t: &Tolerance) {
    let slack = 1e-3 * t.linear;
    for curve in &m.curves {
        assert!(
            curve.deviation >= 0.0 && curve.deviation <= t.linear,
            "{curve:?}"
        );
        assert_eq!(curve.exact, curve.deviation == 0.0);
        if curve.exact {
            assert!(
                !matches!(&curve.curve, Curve3::Nurbs(c) if c.degree() != 2),
                "exact NURBS are conics"
            );
        }
        let points = samples(curve, 240);
        for (k, p) in points.iter().enumerate() {
            let bound = curve.deviation + slack;
            assert!(
                distance(a, *p) <= bound,
                "off a by {} at {k}",
                distance(a, *p)
            );
            assert!(
                distance(b, *p) <= bound,
                "off b by {} at {k}",
                distance(b, *p)
            );
            assert!(
                within.contains(*p, 2.0 * t.linear),
                "outside the box at {k}"
            );
            if k > 2
                && k < 238
                && k % 16 == 5
                && let (Some(na), Some(nb)) = (normal_at(a, *p), normal_at(b, *p))
            {
                let sine = na.cross(nb).length();
                assert_eq!(curve.tangent, sine < 1e-6, "sine {sine} at {k}: {curve:?}");
            }
        }
        let gap = points[0].distance(points[240]);
        if curve.closed {
            assert!(gap <= slack, "a closed curve must close: {gap}");
        }
        let length: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!(length > t.linear);
    }
    for p in &m.points {
        assert!(distance(a, *p) <= 2.0 * t.linear && distance(b, *p) <= 2.0 * t.linear);
    }
}

/// Distance from `p` to the nearest of the meeting's curves and points
/// (curves as polylines of `per_curve` chords).
pub fn distance_to_meeting(m: &Meeting, p: Vec3, per_curve: usize) -> f64 {
    let mut best = f64::INFINITY;
    for curve in &m.curves {
        let points = samples(curve, per_curve);
        for w in points.windows(2) {
            let d = w[1] - w[0];
            let l2 = d.length_squared();
            let s = if l2 > 0.0 {
                ((p - w[0]).dot(d) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            best = best.min((w[0] + d * s).distance(p));
        }
    }
    for q in &m.points {
        best = best.min(q.distance(p));
    }
    best
}

/// A small deterministic generator (SplitMix64): no dependency, the same
/// numbers everywhere.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[a, b)`.
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }

    pub fn vec(&mut self, half: f64) -> Vec3 {
        Vec3::new(
            self.range(-half, half),
            self.range(-half, half),
            self.range(-half, half),
        )
    }

    pub fn frame(&mut self, half: f64) -> Frame {
        loop {
            let (z, x) = (self.vec(1.0), self.vec(1.0));
            if z.length() > 0.2
                && x.length() > 0.2
                && let Some(f) = Frame::new(self.vec(half), z, x)
            {
                return f;
            }
        }
    }
}
