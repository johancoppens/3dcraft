//! Shared by the revolution tests: Pappus's theorems worked out from the
//! sketch curves themselves (not from the kernel's profiles), and axes
//! placed clear of a set of curves.

#![allow(dead_code)]

use crate::common::{P, arc, line, polygon};
use craft3d_kernel::extrude::OpResult;
use craft3d_kernel::extrude::Origin;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Sweeps: a full turn, a quarter, three quarters from a negative angle, a
/// sliver, and nearly a whole turn across angle 0.
pub const SWEEPS: [(f64, f64); 5] = [
    (0.0, TAU),
    (0.0, FRAC_PI_2),
    (-0.7, -0.7 + 1.5 * PI),
    (2.0, 2.3),
    (-3.0, 3.1),
];

/// The modelling tolerance at scale `k` (docs/decisions.md).
pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// An axis in the sketch: a point and a direction (any length).
#[derive(Clone, Copy, Debug)]
pub struct Axis {
    pub point: P,
    pub direction: P,
}

impl Axis {
    /// How far `p` is to the left of the axis.
    pub fn left(&self, p: P) -> f64 {
        let length = self.direction[0].hypot(self.direction[1]);
        let n = [-self.direction[1] / length, self.direction[0] / length];
        (p[0] - self.point[0]) * n[0] + (p[1] - self.point[1]) * n[1]
    }
}

/// The length of a curve and the centroid of the curve itself (of the
/// line, not of what it encloses).
///
/// A line: its midpoint. A circle: its centre. An arc of angle `a`: on the
/// bisector, `r sin(a/2) / (a/2)` from the centre.
pub fn measure(curve: &Curve2) -> (f64, P) {
    match *curve {
        Curve2::Spline(_) => panic!("the revolve tests draw no splines: they are not revolved"),
        Curve2::Line { start, end } => (
            (end[0] - start[0]).hypot(end[1] - start[1]),
            [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0],
        ),
        Curve2::Circle { center, radius } => (TAU * radius, center),
        Curve2::Arc { center, start, end } => {
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let begin = (start[1] - center[1]).atan2(start[0] - center[0]);
            let finish = (end[1] - center[1]).atan2(end[0] - center[0]);
            let angle = (finish - begin).rem_euclid(TAU);
            let reach = radius * (angle / 2.0).sin() / (angle / 2.0);
            let middle = begin + angle / 2.0;
            (
                radius * angle,
                [
                    center[0] + reach * middle.cos(),
                    center[1] + reach * middle.sin(),
                ],
            )
        }
    }
}

/// Corners of a box that holds the curves (an arc counts as its whole
/// circle).
pub fn corners(curves: &[SketchCurve]) -> [P; 4] {
    let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    let mut add = |p: P, r: f64| {
        for k in 0..2 {
            low[k] = low[k].min(p[k] - r);
            high[k] = high[k].max(p[k] + r);
        }
    };
    for c in curves {
        match c.curve {
            Curve2::Line { start, end } => {
                add(start, 0.0);
                add(end, 0.0);
            }
            Curve2::Spline(ref spline) => spline.control.iter().for_each(|p| add(*p, 0.0)),
            Curve2::Circle { center, radius } => add(center, radius),
            Curve2::Arc { center, start, .. } => {
                add(center, (start[0] - center[0]).hypot(start[1] - center[1]))
            }
        }
    }
    [
        [low[0], low[1]],
        [high[0], low[1]],
        [high[0], high[1]],
        [low[0], high[1]],
    ]
}

/// The axis along `direction` that has all the curves to its left, the
/// nearest of their box's corners `gap` away.
pub fn clear_axis(curves: &[SketchCurve], direction: P, gap: f64) -> Axis {
    let length = direction[0].hypot(direction[1]);
    let n = [-direction[1] / length, direction[0] / length];
    let nearest = corners(curves)
        .iter()
        .map(|c| c[0] * n[0] + c[1] * n[1])
        .fold(f64::INFINITY, f64::min);
    Axis {
        point: [n[0] * (nearest - gap), n[1] * (nearest - gap)],
        direction,
    }
}

/// The same line, pointing the other way: the curves are then to its right
/// and the revolution turns the other way.
pub fn opposite(axis: Axis) -> Axis {
    Axis {
        point: axis.point,
        direction: [-axis.direction[0], -axis.direction[1]],
    }
}

/// Volume and total area of the revolution of regions with the given
/// `(area, centroid)`, bounded by `curves`, through `sweep`, by Pappus:
/// a region sweeps its area times the path of its centroid, a curve its
/// length times the path of its own centroid. Less than a full turn adds
/// the two caps.
pub fn pappus(curves: &[SketchCurve], regions: &[(f64, P)], axis: Axis, sweep: f64) -> (f64, f64) {
    let volume: f64 = regions
        .iter()
        .map(|&(area, centroid)| area * axis.left(centroid).abs() * sweep)
        .sum();
    let lateral: f64 = curves
        .iter()
        .map(|c| {
            let (length, centroid) = measure(&c.curve);
            length * axis.left(centroid).abs() * sweep
        })
        .sum();
    let caps = if sweep < TAU - 1e-9 {
        2.0 * regions.iter().map(|r| r.0).sum::<f64>()
    } else {
        0.0
    };
    (volume, lateral + caps)
}

/// Profiles from `curves`, revolved.
pub fn build(
    curves: &[SketchCurve],
    plane: &Frame,
    axis: Axis,
    (from, to): (f64, f64),
    tol: &Tolerance,
) -> Result<OpResult, craft3d_kernel::KernelError> {
    let profiles = detect_profiles(curves, tol)?;
    revolve(&profiles, plane, axis.point, axis.direction, from, to, tol)
}

pub fn close(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual} vs {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

/// Everything that must hold for any revolution: a valid body, an origin
/// for every entity and no origin twice, and Pappus.
pub fn assert_pappus(
    result: &OpResult,
    curves: &[SketchCurve],
    regions: &[(f64, P)],
    axis: Axis,
    sweep: f64,
    k: f64,
    what: &str,
) {
    let body = &result.body;
    assert_eq!(check(body, &tolerance(k)), vec![], "{what}");
    assert_eq!(result.origins.faces.len(), body.faces.len(), "{what}");
    assert_eq!(result.origins.edges.len(), body.edges.len(), "{what}");
    assert_eq!(result.origins.vertices.len(), body.vertices.len(), "{what}");
    for origins in [
        &result.origins.faces,
        &result.origins.edges,
        &result.origins.vertices,
    ] {
        let unique: BTreeSet<&Origin> = origins.iter().collect();
        assert_eq!(unique.len(), origins.len(), "{what}: an origin twice");
    }
    let mass = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    let (volume, area) = pappus(curves, regions, axis, sweep);
    // 1e-10 relative: the integrals are exact to rounding; the body is
    // placed far from the origin on a tilted plane, which costs digits.
    close(mass.volume, volume, 1e-10, &format!("{what}: volume"));
    close(mass.area, area, 1e-10, &format!("{what}: area"));
}

/// A sketch point given in the axis's own terms: `rho` to the left of it
/// (`side` +1) or to its right (−1), and `h` along it.
pub fn lathe(axis: Axis, side: f64, rho: f64, h: f64) -> P {
    let length = axis.direction[0].hypot(axis.direction[1]);
    let d = [axis.direction[0] / length, axis.direction[1] / length];
    let n = [-d[1] * side, d[0] * side];
    [
        axis.point[0] + rho * n[0] + h * d[0],
        axis.point[1] + rho * n[1] + h * d[1],
    ]
}

/// The polygon through points given as (rho, h), ids 1, 2, …
pub fn lathe_polygon(axis: Axis, side: f64, points: &[(f64, f64)]) -> Vec<SketchCurve> {
    let points: Vec<P> = points
        .iter()
        .map(|&(rho, h)| lathe(axis, side, rho, h))
        .collect();
    polygon(1, 1, &points)
}

/// Half a disc of `radius` about the point `h` along the axis, on `side`:
/// the arc (id 1) from pole to pole and the line on the axis (id 2).
pub fn half_disc(axis: Axis, side: f64, h: f64, radius: f64) -> Vec<SketchCurve> {
    let (low, high) = (
        lathe(axis, side, 0.0, h - radius),
        lathe(axis, side, 0.0, h + radius),
    );
    let centre = lathe(axis, side, 0.0, h);
    // Counter-clockwise an arc runs upwards to the right of the axis and
    // downwards to its left.
    if side < 0.0 {
        vec![arc(1, centre, low, high), line(2, high, low)]
    } else {
        vec![arc(1, centre, high, low), line(2, low, high)]
    }
}
