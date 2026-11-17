//! Fillets whose radius varies along the edge by a law
//! (`kernel::fillet::fillet_by_law`): the radius is one along the edge and
//! runs to another at an end, over a length along the edge, by
//! `r = to + (radius − to) (1 − (1 − τ)³)` with `τ` the distance from that
//! end over the length.
//!
//! What the fillet is held to here comes from its definition, worked out
//! in this file on its own: in each plane across the edge, at the length
//! `a` along it, the fillet's section is the arc of the ball of radius
//! `r(a)` that touches both faces, between its two feet.
//!
//! - Between two planes the section is that arc in closed form, so the
//!   volume is the prism's less `(cot(θ/2) − (π − θ)/2) ∫ r² da` and every
//!   point of the fillet face lies on its arc.
//! - Between two spline walls along a straight edge it is the fillet of
//!   the two curves in the plane, with the radius of that plane.
//! - Along a spline rim between a flat top and an upright spline wall it
//!   is a quarter circle in the plane across the rim, and the volume
//!   follows from the rim's curvature.
//! - Between a plane and a cone the ball of each plane is found here by
//!   its own equations.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, boxed, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{
    Fade, FilletEdge, FilletFace, FilletLawEdge, FilletResult, RadiusLaw, fillet, fillet_by_law,
};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// A law as the tests give it: the radius, and the fade at the low end
/// and at the high end of the edge (the radius there, the length), told
/// apart by where the edge's ends lie along a direction.
#[derive(Clone, Copy, Debug)]
struct Law {
    radius: f64,
    low: Option<(f64, f64)>,
    high: Option<(f64, f64)>,
}

impl Law {
    /// The radius at the length `a` from the low end of an edge `length`
    /// long, and its derivative there: the law's definition, written out.
    fn at(&self, a: f64, length: f64) -> (f64, f64) {
        let mut out = (self.radius, 0.0);
        for (fade, from_end, sign) in [(self.low, a, 1.0), (self.high, length - a, -1.0)] {
            let Some((to, over)) = fade else { continue };
            let tau = from_end / over;
            if tau < 1.0 {
                let m = 1.0 - tau;
                out = (
                    to + (self.radius - to) * (1.0 - m * m * m),
                    sign * (self.radius - to) * 3.0 * m * m / over,
                );
            }
        }
        out
    }

    /// The radius at the low and at the high end.
    fn ends(&self) -> [f64; 2] {
        [
            self.low.map_or(self.radius, |f| f.0),
            self.high.map_or(self.radius, |f| f.0),
        ]
    }

    /// `∫ r da` and `∫ r² da` over an edge `length` long, in closed form:
    /// over a fade of length `L` from `to`, with `Δ = radius − to` and
    /// `∫₀¹ f = 3/4`, `∫₀¹ f² = 9/14`, they are `L (to + 3Δ/4)` and
    /// `L (to² + 3 to Δ/2 + 9Δ²/14)`.
    fn integrals(&self, length: f64) -> (f64, f64) {
        let mut plain = length;
        let (mut first, mut second) = (0.0, 0.0);
        for (to, over) in [self.low, self.high].into_iter().flatten() {
            let d = self.radius - to;
            plain -= over;
            first += over * (to + 0.75 * d);
            second += over * (to * to + 1.5 * to * d + 9.0 / 14.0 * d * d);
        }
        (
            first + plain * self.radius,
            second + plain * self.radius * self.radius,
        )
    }

    /// Where the law's pieces join along an edge `length` long.
    fn breaks(&self, length: f64) -> Vec<f64> {
        let mut out = vec![0.0];
        if let Some((_, over)) = self.low {
            out.push(over);
        }
        if let Some((_, over)) = self.high {
            out.push(length - over);
        }
        out.push(length);
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// The law at scale `k` for the edge `edge` of `body`, whose low end
    /// is the one further back along `up`.
    fn on(&self, body: &Body, edge: EdgeId, up: Vec3, k: f64) -> RadiusLaw {
        let data = body.edge(edge).unwrap();
        let (start, end) = (data.curve.eval(data.range.0), data.curve.eval(data.range.1));
        let fade = |f: Option<(f64, f64)>| f.map(|(to, over)| Fade::to(to * k, over * k));
        let fades = if (end - start).dot(up) > 0.0 {
            [fade(self.low), fade(self.high)]
        } else {
            [fade(self.high), fade(self.low)]
        };
        RadiusLaw {
            radius: self.radius * k,
            fades,
        }
    }
}

/// Simpson's rule for `f` over `(a, b)` in `2 n` steps.
fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, n: usize) -> f64 {
    let h = (b - a) / (2 * n) as f64;
    let mut sum = f(a) + f(b);
    for i in 1..2 * n {
        let weight = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += weight * f(a + h * i as f64);
    }
    sum * h / 3.0
}

/// The edge whose middle is nearest `middle` (in `frame`, at scale 1).
fn edge_at(body: &Body, frame: &Frame, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = frame.point(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, off) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    assert!(off < 1e-3 * k, "no edge at {middle:?}: {off}");
    EdgeId(index as u32)
}

fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .filter(|i| {
            body.faces[*i]
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .map(|i| FaceId(i as u32))
        .collect()
}

fn normal_at(body: &Body, face: FaceId, point: Vec3) -> Vec3 {
    let face = body.face(face).unwrap();
    let (u, v) = face.surface.project(point).unwrap();
    let n = face.surface.normal(u, v).unwrap();
    if face.reversed { -n } else { n }
}

/// The one fillet face of a result.
fn fillet_face(made: &FilletResult) -> FaceId {
    let found: Vec<usize> = (0..made.faces.len())
        .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
        .collect();
    assert_eq!(found.len(), 1, "one fillet face");
    FaceId(found[0] as u32)
}

/// The largest sine of the angle between the normals of the fillet face
/// and each face it runs into along a border, at 33 points of each of its
/// two borders (the edges it shares with a face whose normal agrees with
/// its own to 1e-3: the cuts at its ends are creases by design).
fn crease(made: &FilletResult) -> f64 {
    let body = &made.body;
    let own = fillet_face(made);
    let mut worst = 0.0_f64;
    let mut borders = 0;
    for (index, edge) in body.edges.iter().enumerate() {
        let users = faces_of(body, EdgeId(index as u32));
        let [a, b] = users[..] else { continue };
        if (a == own) == (b == own) {
            continue;
        }
        let mut here = 0.0_f64;
        for i in 0..33 {
            let s = (f64::from(i) + 0.5) / 33.0;
            let p = edge
                .curve
                .eval(edge.range.0 + s * (edge.range.1 - edge.range.0));
            let (n, m) = (normal_at(body, a, p), normal_at(body, b, p));
            here = here.max(n.cross(m).length());
        }
        if here < 1e-3 {
            borders += 1;
            worst = worst.max(here);
        }
    }
    assert_eq!(borders, 2, "the two borders of the fillet");
    worst
}

/// Valid, and meshed watertight with the volume it encloses.
fn sound(body: &Body, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let exact = mass(body);
    let chord = 0.01;
    let params = TessParams {
        chordal_tolerance: chord * k,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - exact.volume).abs() <= exact.area * chord * k,
        "{what}: mesh volume {} against {}",
        welded.volume(),
        exact.volume
    );
}

/// Points of the fillet face's own surface, over the part of it along the
/// edge: a grid of its parameters.
fn points_of(made: &FilletResult) -> Vec<Vec3> {
    let face = made.body.face(fillet_face(made)).unwrap();
    assert!(matches!(face.surface, Surface::Nurbs(_)), "a fitted face");
    let ((u0, u1), (v0, v1)) = face.surface.domain();
    let mut out = Vec::new();
    for i in 0..=96 {
        for j in 0..=8 {
            let u = u0 + (u1 - u0) * f64::from(i) / 96.0;
            let v = v0 + (v1 - v0) * f64::from(j) / 8.0;
            out.push(face.surface.eval(u, v));
        }
    }
    out
}

/// A prism over a triangle with the corner `corner` (the angle inside the
/// material, in degrees) at the origin, or, `concave`, over an L whose
/// inside corner is at the origin; 40 tall. The upright edge at the
/// origin is filleted by `law`, low end at the bottom.
///
/// In the plane at height `z` the section is the arc of radius `r(z)`
/// that touches both planes: its centre `r / sin(θ/2)` from the edge on
/// the bisector, its feet `r cot(θ/2)` from the edge, and what it takes
/// from the corner `(cot(θ/2) − (π − θ)/2) r²`. So the volume changes by
/// that times `∫ r² dz`, each side face loses `cot(θ/2) ∫ r dz`, each end
/// face changes by the section at its end, and the fillet face is the
/// surface `c(z) + r(z) u(α)`, whose area is integrated here.
fn between_planes(corner: f64, concave: bool, law: Law) {
    let height = 40.0;
    let theta = corner.to_radians();
    let half = 0.5 * theta;
    let cot = half.pcos() / half.psin();
    let section = cot - 0.5 * (PI - theta);
    let (first, second) = law.integrals(height);
    // The fillet face's area: |S_z × S_α| over the height and the arc.
    let bisector = [half.pcos(), half.psin()];
    let across = [-half.psin(), half.pcos()];
    let span = 0.5 * (PI - theta);
    let element = |z: f64, alpha: f64| {
        let (r, dr) = law.at(z, height);
        let (sin, cos) = (alpha.psin(), alpha.pcos());
        let u = [
            -bisector[0] * cos + across[0] * sin,
            -bisector[1] * cos + across[1] * sin,
        ];
        let s_z = Vec3::new(
            dr * (bisector[0] / half.psin() + u[0]),
            dr * (bisector[1] / half.psin() + u[1]),
            1.0,
        );
        let s_a = Vec3::new(
            r * (bisector[0] * sin + across[0] * cos),
            r * (bisector[1] * sin + across[1] * cos),
            0.0,
        );
        s_z.cross(s_a).length()
    };
    let mut fillet_area = 0.0;
    for piece in law.breaks(height).windows(2) {
        fillet_area += simpson(
            |z| simpson(|alpha| element(z, alpha), -span, span, 48),
            piece[0],
            piece[1],
            200,
        );
    }
    let curves = if concave {
        common::polygon(
            1,
            1,
            &[
                [0.0, 0.0],
                [0.0, -30.0],
                [40.0, -30.0],
                [40.0, 40.0],
                [-30.0, 40.0],
                [-30.0, 0.0],
            ],
        )
    } else {
        common::polygon(
            1,
            1,
            &[
                [0.0, 0.0],
                [40.0, 0.0],
                [40.0 * theta.pcos(), 40.0 * theta.psin()],
            ],
        )
    };
    let [low, high] = law.ends();
    for k in SCALES {
        let tol = tolerance(k);
        let profiles = detect_profiles(&common::scale_curves(&curves, k), &tol).unwrap();
        for (index, frame) in common::frames(k).iter().enumerate() {
            let what = format!("corner {corner}, concave {concave}, scale {k}, frame {index}");
            let body = extrude(&profiles, frame, 0.0, height * k, &tol)
                .unwrap()
                .body;
            let edge = edge_at(&body, frame, k, [0.0, 0.0, 0.5 * height]);
            let made = fillet_by_law(
                &body,
                &[FilletLawEdge {
                    edge,
                    law: law.on(&body, edge, frame.z(), k),
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_eq!(made.body.faces.len(), body.faces.len() + 1, "{what}");
            let (before, after) = (mass(&body), mass(&made.body));
            // Material goes at a convex corner and comes at a concave one.
            let sign = if concave { 1.0 } else { -1.0 };
            let volume = before.volume + sign * section * second * k * k * k;
            let area = before.area
                + (fillet_area - 2.0 * cot * first + sign * section * (low * low + high * high))
                    * k
                    * k;
            // The fitted face is within a quarter of the tolerance of the
            // arcs all over it: the volume is held to a tenth of what that
            // allows (the face's area times the tolerance).
            let slack = fillet_area * k * k * tol.linear;
            assert!(
                (after.volume - volume).abs() <= 0.1 * slack,
                "{what}: volume {} against {volume}, off by {:e} of {slack:e}",
                after.volume,
                after.volume - volume
            );
            assert!(
                (after.area - area).abs() <= 1e-8 * area,
                "{what}: area {} against {area}",
                after.area
            );
            // Every point of the face on the arc of its own plane.
            let mut held = 0;
            let mut furthest = 0.0_f64;
            for p in points_of(&made) {
                let (x, y, z) = frame.local(p);
                let (x, y, z) = (x / k, y / k, z / k);
                if !(0.0..=height).contains(&z) {
                    continue;
                }
                let (r, _) = law.at(z, height);
                let from_edge = r / half.psin();
                let centre = if concave {
                    [-from_edge * bisector[0], -from_edge * bisector[1]]
                } else {
                    [from_edge * bisector[0], from_edge * bisector[1]]
                };
                let off = (x - centre[0]).phypot(y - centre[1]) - r;
                furthest = furthest.max(off.abs() * k / tol.linear);
                assert!(
                    off.abs() * k <= 0.25 * tol.linear,
                    "{what}: a point at height {z} is {off:e} off its arc"
                );
                // Between the feet: no further from the edge than they.
                assert!(
                    x.phypot(y) <= r * cot + 1e-6,
                    "{what}: a point at height {z} beyond the feet"
                );
                held += 1;
            }
            assert!(held > 500, "{what}: only {held} points looked at");
            // Tangent to both faces along its borders.
            let worst = crease(&made);
            assert!(
                worst <= tol.linear / (law.radius * k),
                "{what}: a crease of {worst:e}"
            );
            println!(
                "{what}: volume off {:.3} of the slack, area off {:.1e} of it, points within {furthest:.3} tolerances, crease {worst:.1e}, {} faces",
                (after.volume - volume).abs() / slack,
                (after.area - area).abs() / area,
                made.body.faces.len()
            );
        }
    }
}

#[test]
fn a_law_between_two_planes_at_a_right_angle_is_the_arc_of_each_height() {
    // 3 along the edge, from 1 over 15 at the bottom to 4.5 over 10 at the
    // top.
    between_planes(
        90.0,
        false,
        Law {
            radius: 3.0,
            low: Some((1.0, 15.0)),
            high: Some((4.5, 10.0)),
        },
    );
}

#[test]
fn a_law_between_two_planes_at_sixty_degrees_is_the_arc_of_each_height() {
    between_planes(
        60.0,
        false,
        Law {
            radius: 2.0,
            low: Some((3.0, 12.0)),
            high: Some((0.8, 20.0)),
        },
    );
}

#[test]
fn a_law_between_two_planes_at_a_hundred_and_twenty_degrees_is_the_arc_of_each_height() {
    // One fade only, over the whole edge.
    between_planes(
        120.0,
        false,
        Law {
            radius: 5.0,
            low: None,
            high: Some((1.5, 40.0)),
        },
    );
}

#[test]
fn a_law_along_an_inside_corner_adds_the_arc_of_each_height() {
    between_planes(
        90.0,
        true,
        Law {
            radius: 4.0,
            low: Some((1.0, 18.0)),
            high: Some((2.5, 22.0)),
        },
    );
}

#[test]
fn a_law_that_nearly_runs_to_nothing_is_still_the_arc_of_each_height() {
    // From 3 down to a hundredth of it at the top, over 15.
    between_planes(
        90.0,
        false,
        Law {
            radius: 3.0,
            low: None,
            high: Some((0.03, 15.0)),
        },
    );
}

#[test]
fn a_law_that_changes_fast_is_still_the_arc_of_each_height() {
    // From 1 up to 3 within 3 of the bottom (the radius changes twice as
    // fast as the length there), and on to 4 within 1 of the top (three
    // times as fast).
    between_planes(
        90.0,
        false,
        Law {
            radius: 3.0,
            low: Some((1.0, 3.0)),
            high: Some((4.0, 1.0)),
        },
    );
}

/// A cubic Bézier in the plane.
#[derive(Clone, Copy)]
struct Bez([[f64; 2]; 4]);

impl Bez {
    fn at(&self, t: f64) -> [f64; 2] {
        let s = 1.0 - t;
        let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        let p = self.0;
        [
            b[0] * p[0][0] + b[1] * p[1][0] + b[2] * p[2][0] + b[3] * p[3][0],
            b[0] * p[0][1] + b[1] * p[1][1] + b[2] * p[2][1] + b[3] * p[3][1],
        ]
    }
    fn d(&self, t: f64) -> [f64; 2] {
        let s = 1.0 - t;
        let p = self.0;
        let c = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
        let q = |i: usize, k: usize| p[i + 1][k] - p[i][k];
        [
            c[0] * q(0, 0) + c[1] * q(1, 0) + c[2] * q(2, 0),
            c[0] * q(0, 1) + c[1] * q(1, 1) + c[2] * q(2, 1),
        ]
    }
    /// The nearest point's parameter, by a search and Newton.
    fn nearest(&self, p: [f64; 2]) -> f64 {
        let mut best = (0.0, f64::INFINITY);
        for i in 0..=400 {
            let t = f64::from(i) / 400.0;
            let q = self.at(t);
            let d = (q[0] - p[0]).phypot(q[1] - p[1]);
            if d < best.1 {
                best = (t, d);
            }
        }
        let mut t = best.0;
        for _ in 0..50 {
            let (q, dq) = (self.at(t), self.d(t));
            let h = 1e-6;
            let (q2, dq2) = (self.at(t + h), self.d(t + h));
            let f = (q[0] - p[0]) * dq[0] + (q[1] - p[1]) * dq[1];
            let f2 = (q2[0] - p[0]) * dq2[0] + (q2[1] - p[1]) * dq2[1];
            let step = f * h / (f2 - f);
            t = (t - step).clamp(0.0, 1.0);
            if step.abs() < 1e-15 {
                break;
            }
        }
        t
    }
    /// Half of the integral of x dy − y dx from `a` to `b` (Gauss-Legendre,
    /// exact for the polynomial it is).
    fn green(&self, a: f64, b: f64) -> f64 {
        let nodes = [
            (-0.906_179_845_938_664, 0.236_926_885_056_189),
            (-0.538_469_310_105_683, 0.478_628_670_499_366),
            (0.0, 0.568_888_888_888_889),
            (0.538_469_310_105_683, 0.478_628_670_499_366),
            (0.906_179_845_938_664, 0.236_926_885_056_189),
        ];
        let mut sum = 0.0;
        for (x, w) in nodes {
            let t = 0.5 * (a + b) + 0.5 * (b - a) * x;
            let (p, d) = (self.at(t), self.d(t));
            sum += w * (p[0] * d[1] - p[1] * d[0]);
        }
        0.5 * sum * 0.5 * (b - a)
    }
}

/// A blade's section of two cubic splines, extruded: its leading edge is a
/// straight edge between two spline walls. In the plane at each height
/// the fillet is the 2D fillet of the two curves with the radius of that
/// height, worked out here on its own (the ball's centre the radius in
/// from the first curve, moved along it until it is the radius from the
/// second; what it takes by Green's theorem over the curves and the arc),
/// and the volume it takes is that area integrated over the height.
#[test]
fn a_law_between_two_spline_walls_is_the_fillet_of_the_curves_at_each_height() {
    let suction = Bez([
        [-7.0421, 42.7558],
        [-8.9846, 43.7886],
        [-7.8518, 55.0783],
        [7.0421, 69.2442],
    ]);
    let pressure = Bez([
        [7.0421, 69.2442],
        [-1.852, 55.2859],
        [-5.1711, 42.4405],
        [-7.0421, 42.7558],
    ]);
    let height = 5.0;
    let law = Law {
        radius: 0.3,
        low: Some((0.15, 2.0)),
        high: Some((0.45, 1.5)),
    };
    // What the fillet of radius `r` takes from the section.
    let taken = |r: f64| -> f64 {
        let inward = |t: f64| {
            let d = suction.d(t);
            let len = d[0].phypot(d[1]);
            // The section runs clockwise, suction then pressure: in is to
            // the right of the suction side.
            [d[1] / len, -d[0] / len]
        };
        let centre_at = |t: f64| {
            let (p, n) = (suction.at(t), inward(t));
            [p[0] + r * n[0], p[1] + r * n[1]]
        };
        let gap = |t: f64| {
            let m = centre_at(t);
            let q = pressure.at(pressure.nearest(m));
            (q[0] - m[0]).phypot(q[1] - m[1]) - r
        };
        let (mut lo, mut hi) = (1e-4, 0.5);
        assert!(gap(lo) < 0.0 && gap(hi) > 0.0);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if gap(mid) < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let t1 = 0.5 * (lo + hi);
        let m = centre_at(t1);
        let t2 = pressure.nearest(m);
        let (q1, q2) = (suction.at(t1), pressure.at(t2));
        let a1 = (q1[1] - m[1]).patan2(q1[0] - m[0]);
        let wrap = |a: f64| (a + PI).rem_euclid(2.0 * PI) - PI;
        let a2 = a1 + wrap((q2[1] - m[1]).patan2(q2[0] - m[0]) - a1);
        let arc = r * r * (a2 - a1)
            + r * (m[0] * (a2.psin() - a1.psin()) - m[1] * (a2.pcos() - a1.pcos()));
        (suction.green(0.0, t1) + 0.5 * arc + pressure.green(t2, 1.0)).abs()
    };
    let mut removed = 0.0;
    for piece in law.breaks(height).windows(2) {
        removed += simpson(|z| taken(law.at(z, height).0), piece[0], piece[1], 16);
    }
    // One radius all along takes the area of that radius times the
    // height: the integral is that for a law that does not change.
    let flat = Law {
        radius: 0.3,
        low: None,
        high: None,
    };
    assert!(
        (simpson(|z| taken(flat.at(z, height).0), 0.0, height, 4) - taken(0.3) * height).abs()
            < 1e-12
    );
    let corner = suction.at(0.0);
    let spline = |id: u64, b: &Bez, k: f64| SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: b.0.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
        }),
    };
    for k in SCALES {
        let tol = tolerance(k);
        let curves = [spline(1, &suction, k), spline(2, &pressure, k)];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        for (index, frame) in common::frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let body = extrude(&profiles, frame, 0.0, height * k, &tol)
                .unwrap()
                .body;
            let edge = edge_at(&body, frame, k, [corner[0], corner[1], 0.5 * height]);
            for face in faces_of(&body, edge) {
                let surface = &body.face(face).unwrap().surface;
                assert!(matches!(surface, Surface::Nurbs(_) | Surface::Bezier(_)));
            }
            let made = fillet_by_law(
                &body,
                &[FilletLawEdge {
                    edge,
                    law: law.on(&body, edge, frame.z(), k),
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            let took = mass(&body).volume - mass(&made.body).volume;
            let expected = removed * k * k * k;
            // The fitted face is within a quarter of the tolerance of the
            // arcs all over it, and no arc is longer than half a turn.
            let slack = PI * law.integrals(height).0 * k * k * tol.linear;
            println!(
                "{what}: took {took} against {expected}, off {:.3} of the slack",
                (took - expected).abs() / slack
            );
            assert!(
                (took - expected).abs() <= 0.1 * slack,
                "{what}: took {took} instead of {expected}"
            );
            let worst = crease(&made);
            assert!(
                worst <= tol.linear / (law.ends()[0] * k),
                "{what}: a crease of {worst:e}"
            );
        }
    }
}

/// The rim of a prism whose one wall is a spline: a curved edge between
/// the flat top and an upright freeform wall, ending where the wall meets
/// the prism's flat sides square on. The plane across the rim at a point
/// holds the wall's normal there and the upright direction, so the ball
/// of radius `r` in it sits `r` in from the wall and `r` under the top
/// and touches both in that plane: the section is a quarter circle. With
/// `κ` the rim's curvature towards the material, a region of the plane
/// across the rim sweeps the volume `∫ (A − κ M) da` (`A` its area, `M`
/// its moment about the wall), and for what a quarter circle takes from
/// the corner `A = (1 − π/4) r²` and `M = (5/6 − π/4) r³`. The top loses
/// the strip `∫ (r − κ r²/2) da`, the wall `∫ r da`, each flat side the
/// section at its end, and the fillet face is integrated from its points
/// `C + r (1 − sin α) m − r (1 − cos α) z`.
#[test]
fn a_law_along_a_spline_rim_is_a_quarter_circle_across_it() {
    use craft3d_geom::curvature::curve_second_derivative;
    use craft3d_geom::{Curve3, NurbsCurve};
    let control = [
        [30.0, 10.0],
        [25.0, 10.0],
        [20.0, 13.0],
        [12.0, 8.0],
        [5.0, 10.0],
        [0.0, 10.0],
    ];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    let height = 8.0;
    let law = Law {
        radius: 1.5,
        low: Some((0.6, 9.0)),
        high: Some((2.2, 7.0)),
    };
    // The rim at scale 1, in the plane: its points, speed and curvature
    // towards the material on a fine grid, and the length to each.
    let rim = Curve3::Nurbs(
        NurbsCurve::new(
            3,
            knots.clone(),
            control.iter().map(|p| Vec3::new(p[0], p[1], 0.0)).collect(),
            vec![1.0; 6],
        )
        .unwrap(),
    );
    let speed = |t: f64| rim.derivatives(t).1.length();
    let steps = 3000_usize;
    let grid: Vec<f64> = (0..=2 * steps)
        .map(|i| i as f64 / (2 * steps) as f64)
        .collect();
    let mut lengths = vec![0.0; grid.len()];
    for i in 1..=steps {
        let (a, m, b) = (grid[2 * i - 2], grid[2 * i - 1], grid[2 * i]);
        let whole = (b - a) / 6.0 * (speed(a) + 4.0 * speed(m) + speed(b));
        let first = (m - a) / 6.0 * (speed(a) + 4.0 * speed(0.5 * (a + m)) + speed(m));
        lengths[2 * i - 1] = lengths[2 * i - 2] + first;
        lengths[2 * i] = lengths[2 * i - 2] + whole;
    }
    let length = *lengths.last().unwrap();
    // Into the material: the outline runs counter-clockwise, so the
    // material is on the left of the rim.
    let frame_at = |t: f64| {
        let (point, d) = rim.derivatives(t);
        let tangent = d.normalized().unwrap();
        let inward = Vec3::new(-tangent.y, tangent.x, 0.0);
        let d2 = curve_second_derivative(&rim, t).unwrap();
        let fourth = d.length_squared() * d.length_squared();
        let bend = d.cross(d2).cross(d) * (1.0 / fourth);
        (point, tangent, inward, bend.dot(inward))
    };
    let quarter = 1.0 - 0.25 * PI;
    let moment = 5.0 / 6.0 - 0.25 * PI;
    // Simpson over the grid of what `f` gives per unit of length.
    let along = |f: &dyn Fn(f64, f64, f64) -> f64| -> f64 {
        let mut sum = 0.0;
        for (i, t) in grid.iter().enumerate() {
            let weight = if i == 0 || i == 2 * steps {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            let (r, dr) = law.at(lengths[i], length);
            let (_, _, _, kappa) = frame_at(*t);
            let _ = dr;
            sum += weight * f(lengths[i], r, kappa) * speed(*t);
        }
        sum / (6 * steps) as f64
    };
    let removed = along(&|_, r, kappa| quarter * r * r - kappa * moment * r * r * r);
    let top_lost = along(&|_, r, kappa| r - 0.5 * kappa * r * r);
    let wall_lost = along(&|_, r, _| r);
    let fillet_area = along(&|a, r, kappa| {
        let (_, dr) = law.at(a, length);
        simpson(
            |alpha| {
                let (sin, cos) = (alpha.psin(), alpha.pcos());
                let s_a = Vec3::new(
                    1.0 - kappa * r * (1.0 - sin),
                    dr * (1.0 - sin),
                    -dr * (1.0 - cos),
                );
                let s_alpha = Vec3::new(0.0, -r * cos, -r * sin);
                s_a.cross(s_alpha).length()
            },
            0.0,
            0.5 * PI,
            24,
        )
    });
    let [low, high] = law.ends();
    let middle = rim.eval(0.5);
    for k in SCALES {
        let tol = tolerance(k);
        let scaled = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: scaled(a),
                end: scaled(b),
            },
        };
        let curves = [
            line(1, [0.0, 0.0], [30.0, 0.0]),
            line(2, [30.0, 0.0], [30.0, 10.0]),
            SketchCurve {
                id: 3,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: knots.clone(),
                    control: control.iter().map(|p| scaled(*p)).collect(),
                }),
            },
            line(4, [0.0, 10.0], [0.0, 0.0]),
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        for (index, frame) in common::frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let body = extrude(&profiles, frame, 0.0, height * k, &tol)
                .unwrap()
                .body;
            let edge = edge_at(&body, frame, k, [middle.x, middle.y, height]);
            assert!(matches!(body.edge(edge).unwrap().curve, Curve3::Nurbs(_)));
            // The low end is the rim's start, at x = 30.
            let made = fillet_by_law(
                &body,
                &[FilletLawEdge {
                    edge,
                    law: law.on(&body, edge, -frame.x(), k),
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            let (before, after) = (mass(&body), mass(&made.body));
            let volume = before.volume - removed * k * k * k;
            let area = before.area
                + (fillet_area - top_lost - wall_lost - quarter * (low * low + high * high))
                    * k
                    * k;
            let slack = fillet_area * k * k * tol.linear;
            // Every point of the face on the quarter circle of its plane.
            let mut held = 0;
            let mut furthest = 0.0_f64;
            for p in points_of(&made) {
                let (x, y, z) = frame.local(p);
                let q = Vec3::new(x / k, y / k, 0.0);
                // The plane across the rim that holds the point.
                let mut best = (0_usize, f64::INFINITY);
                for (i, t) in grid.iter().enumerate().step_by(8) {
                    let d = rim.eval(*t).distance(q);
                    if d < best.1 {
                        best = (i, d);
                    }
                }
                let mut t = grid[best.0];
                for _ in 0..30 {
                    let (c, d) = rim.derivatives(t);
                    let d2 = curve_second_derivative(&rim, t).unwrap();
                    let f = (q - c).dot(d);
                    let df = (q - c).dot(d2) - d.dot(d);
                    t -= f / df;
                }
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let cell = ((t * (2 * steps) as f64) as usize).min(2 * steps - 1);
                let from = grid[cell];
                let a = lengths[cell]
                    + (t - from) / 6.0 * (speed(from) + 4.0 * speed(0.5 * (from + t)) + speed(t));
                let (r, _) = law.at(a, length);
                let (c, _, inward, _) = frame_at(t);
                let centre = c + inward * r + Vec3::new(0.0, 0.0, height - r);
                let off = Vec3::new(q.x, q.y, z / k).distance(centre) - r;
                furthest = furthest.max(off.abs() * k / tol.linear);
                assert!(
                    off.abs() * k <= 0.25 * tol.linear,
                    "{what}: a point at length {a} is {off:e} off its arc"
                );
                held += 1;
            }
            assert!(held > 500, "{what}: only {held} points looked at");
            let worst = crease(&made);
            println!(
                "{what}: volume off {:.3} of the slack, area off {:.1e} of it, points within {furthest:.3} tolerances, crease {worst:.1e}",
                (after.volume - volume).abs() / slack,
                (after.area - area).abs() / area,
            );
            assert!(
                (after.volume - volume).abs() <= 0.1 * slack,
                "{what}: volume {} against {volume}",
                after.volume
            );
            assert!(
                (after.area - area).abs() <= 1e-8 * area,
                "{what}: area {} against {area}",
                after.area
            );
            assert!(
                worst <= tol.linear / (law.ends()[0] * k),
                "{what}: a crease of {worst:e}"
            );
        }
    }
}

/// Half a frustum: the cone of radius 1 and height 3 about z, cut at
/// z = 1 and kept where y ≥ 0.
fn half_frustum(k: f64) -> Body {
    let cone = primitives::cone(&Frame::WORLD, k, 3.0 * k).unwrap();
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 1.0]);
    boolean(&cone, &half, BooleanOp::Intersect, &tolerance(k))
        .unwrap()
        .body
}

/// A straight edge between a plane and a cone (the cut of the half
/// frustum), slanted against both its ends. The plane across the edge at
/// the length `a` holds the centre of the ball of radius `r(a)`, which is
/// `r` behind the flat side and `r` behind the cone: found here from
/// those two equations. Its foot on the cone lies on another ruling than
/// the edge, off that plane, so the arc between the feet lies in the
/// plane through the centre and the two feet. Every point of the fillet
/// face lies on the arc of some length along the edge: the one whose
/// plane holds it, found here, and on that ball.
#[test]
fn a_law_between_a_plane_and_a_cone_is_the_arc_between_the_feet_of_each_ball() {
    let law = Law {
        radius: 0.15,
        low: Some((0.08, 0.4)),
        high: Some((0.2, 0.3)),
    };
    let root = 10f64.sqrt();
    let length = root / 3.0;
    let along = Vec3::new(-1.0, 0.0, 3.0) * (1.0 / root);
    let into = Vec3::new(-3.0, 0.0, -1.0) * (1.0 / root);
    // How far behind the cone's side a point is.
    let behind = |p: Vec3| (1.0 - p.z / 3.0 - p.x.phypot(p.y)) * 3.0 / root;
    // The ball of the plane across the edge at the length `a`: its
    // radius, its centre (`r` behind the flat side, and as far into the
    // cone, across the edge, as puts it `r` behind the cone) and the
    // normal of the plane through the centre and its two feet.
    let ball = |a: f64| -> (f64, Vec3, Vec3) {
        let (r, _) = law.at(a, length);
        let on_edge = Vec3::new(1.0, 0.0, 0.0) + along * a;
        let centre = |b: f64| on_edge + Vec3::new(0.0, r, 0.0) + into * b;
        let (mut lo, mut hi) = (0.0, 5.0 * r);
        assert!(behind(centre(lo)) < r && behind(centre(hi)) > r);
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if behind(centre(mid)) < r {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let c = centre(0.5 * (lo + hi));
        // Out of the cone at the centre's own ruling.
        let rho = c.x.phypot(c.y);
        let out = Vec3::new(3.0 * c.x / rho, 3.0 * c.y / rho, 1.0) * (1.0 / root);
        let flat = Vec3::new(0.0, -1.0, 0.0);
        (r, c, flat.cross(out).normalized().unwrap())
    };
    for k in SCALES {
        let tol = tolerance(k);
        let what = format!("scale {k}");
        let body = half_frustum(k);
        let edge = edge_at(&body, &Frame::WORLD, k, [5.0 / 6.0, 0.0, 0.5]);
        let made = fillet_by_law(
            &body,
            &[FilletLawEdge {
                edge,
                law: law.on(&body, edge, Vec3::Z, k),
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("{what}: {e}"));
        sound(&made.body, k, &what);
        assert!(mass(&made.body).volume < mass(&body).volume, "{what}");
        let mut held = 0;
        let mut furthest = 0.0_f64;
        for p in points_of(&made) {
            let q = p * (1.0 / k);
            // The length whose arc's plane holds the point: from the
            // plane across the edge through it, by the secant rule.
            let off_plane = |a: f64| {
                let (_, c, normal) = ball(a);
                (q - c).dot(normal)
            };
            let mut a = (q - Vec3::new(1.0, 0.0, 0.0)).dot(along);
            if !(-0.05..=length + 0.05).contains(&a) {
                continue;
            }
            let mut before = a - 1e-3;
            for _ in 0..40 {
                let (f, g) = (off_plane(a), off_plane(before));
                if f == g {
                    break;
                }
                let next = a - f * (a - before) / (f - g);
                before = a;
                a = next;
                if (a - before).abs() <= 1e-14 {
                    break;
                }
            }
            if !(0.0..=length).contains(&a) {
                continue;
            }
            assert!(off_plane(a).abs() <= 1e-10, "{what}: no plane found");
            let (r, c, _) = ball(a);
            let off = q.distance(c) - r;
            furthest = furthest.max(off.abs() * k / tol.linear);
            assert!(
                off.abs() * k <= 0.25 * tol.linear,
                "{what}: a point at length {a} is {off:e} off its ball"
            );
            held += 1;
        }
        assert!(held > 400, "{what}: only {held} points looked at");
        let worst = crease(&made);
        println!("{what}: points within {furthest:.3} tolerances, crease {worst:.1e}");
        assert!(
            worst <= tol.linear / (law.ends()[0] * k),
            "{what}: a crease of {worst:e}"
        );
    }
}

/// A law that does not change the radius is the fillet of that radius,
/// the same body to the last bit: between planes (built with an exact
/// cylinder) and between a plane and a cone (a fitted face).
#[test]
fn a_law_of_one_radius_is_the_fillet_of_that_radius() {
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, [0.0, 0.0, 0.0], [40.0, 30.0, 20.0]);
        let upright = edge_at(&block, &Frame::WORLD, k, [0.0, 0.0, 10.0]);
        let frustum = half_frustum(k);
        let cut = edge_at(&frustum, &Frame::WORLD, k, [5.0 / 6.0, 0.0, 0.5]);
        for (body, edge, radius) in [(&block, upright, 3.0 * k), (&frustum, cut, 0.15 * k)] {
            let plain = fillet(body, &[FilletEdge { edge, radius }], &tol).unwrap();
            let same = [
                RadiusLaw::constant(radius),
                RadiusLaw {
                    radius,
                    fades: [Some(Fade::to(radius, 0.1 * k)), None],
                },
            ];
            for law in same {
                let by_law = fillet_by_law(body, &[FilletLawEdge { edge, law }], &tol).unwrap();
                assert!(by_law == plain, "scale {k}: not the same body");
            }
        }
    }
}

/// What the fillet says of a law that is not one, and of a varying radius
/// where it is not built yet: an error, never a body.
#[test]
fn a_law_that_is_not_one_or_not_alone_is_refused() {
    let k = 1.0;
    let tol = tolerance(k);
    let block = boxed(k, [0.0, 0.0, 0.0], [40.0, 30.0, 20.0]);
    let upright = edge_at(&block, &Frame::WORLD, k, [0.0, 0.0, 10.0]);
    let law = |radius: f64, start: Option<(f64, f64)>, end: Option<(f64, f64)>| RadiusLaw {
        radius,
        fades: [start, end].map(|f| f.map(|(to, over)| Fade::to(to, over))),
    };
    let try_one = |body: &Body, edge: EdgeId, law: RadiusLaw| {
        fillet_by_law(body, &[FilletLawEdge { edge, law }], &tol)
    };
    // Numbers that are not sizes.
    for bad in [0.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        // (A fade to nothing, exactly, is a tip: `fillet_tip.rs`.)
        let to = if bad == 0.0 { -1.0 } else { bad };
        for law in [
            law(bad, None, None),
            law(bad, Some((1.0, 5.0)), None),
            law(3.0, Some((to, 5.0)), None),
            law(3.0, None, Some((1.0, bad))),
        ] {
            let refused = try_one(&block, upright, law);
            assert!(
                matches!(refused, Err(KernelError::InvalidSize { .. })),
                "{law:?}: {refused:?}"
            );
        }
    }
    let said = |result: Result<FilletResult, KernelError>, part: &str| match result {
        Err(KernelError::Fillet { reason, .. }) => {
            assert!(reason.contains(part), "{reason}");
        }
        other => panic!("not refused as a fillet: {other:?}"),
    };
    // Longer than the edge, and two fades that overlap.
    said(
        try_one(&block, upright, law(3.0, Some((1.0, 20.001)), None)),
        "longer than the edge",
    );
    said(
        try_one(
            &block,
            upright,
            law(3.0, Some((1.0, 12.0)), Some((2.0, 8.001))),
        ),
        "longer than the edge",
    );
    // The whole edge, and two fades that meet: built.
    for law in [
        law(3.0, Some((1.0, 20.0)), None),
        law(3.0, Some((1.0, 12.0)), Some((2.0, 8.0))),
    ] {
        let made = try_one(&block, upright, law).unwrap_or_else(|e| panic!("{law:?}: {e}"));
        assert_eq!(check(&made.body, &tol), []);
    }
    // An edge that is none, and one given twice.
    assert!(try_one(&block, EdgeId(999), law(3.0, Some((1.0, 5.0)), None)).is_err());
    let twice = FilletLawEdge {
        edge: upright,
        law: law(3.0, Some((1.0, 5.0)), None),
    };
    assert!(fillet_by_law(&block, &[twice, twice], &tol).is_err());
    // Two that meet in a corner, one of them varying.
    let level = edge_at(&block, &Frame::WORLD, k, [20.0, 0.0, 20.0]);
    said(
        fillet_by_law(
            &block,
            &[
                FilletLawEdge {
                    edge: upright,
                    law: law(3.0, Some((1.0, 5.0)), None),
                },
                FilletLawEdge {
                    edge: level,
                    law: RadiusLaw::constant(3.0),
                },
            ],
            &tol,
        ),
        "on its own",
    );
    // Round a closed edge: the rim of a rod.
    let rod = boolean_common::rod(k, [0.0, 0.0, 0.0], 10.0, 20.0);
    let rim = (0..rod.edges.len())
        .map(|i| EdgeId(i as u32))
        .find(|e| {
            let data = rod.edge(*e).unwrap();
            data.start == data.end && data.curve.eval(data.range.0).z > 10.0
        })
        .unwrap();
    said(
        try_one(&rod, rim, law(2.0, Some((1.0, 5.0)), None)),
        "on its own",
    );
    // With an edge elsewhere of one radius: both built, each what it is
    // alone.
    let far = edge_at(&block, &Frame::WORLD, k, [40.0, 30.0, 10.0]);
    let varying = law(3.0, Some((1.0, 12.0)), Some((2.0, 8.0)));
    let both = fillet_by_law(
        &block,
        &[
            FilletLawEdge {
                edge: upright,
                law: varying,
            },
            FilletLawEdge {
                edge: far,
                law: RadiusLaw::constant(2.0),
            },
        ],
        &tol,
    )
    .unwrap();
    assert_eq!(check(&both.body, &tol), []);
    let alone = try_one(&block, upright, varying).unwrap();
    let round = fillet(
        &block,
        &[FilletEdge {
            edge: far,
            radius: 2.0,
        }],
        &tol,
    )
    .unwrap();
    let whole = mass(&block).volume;
    let took = |made: &FilletResult| whole - mass(&made.body).volume;
    assert!((took(&both) - took(&alone) - took(&round)).abs() <= 1e-9 * took(&both));
}
