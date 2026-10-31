//! Fillets and chamfers whose face has no closed form (kernel::blend's
//! `rolling`): a straight edge between a plane and a cone (a conical
//! casing cut away along its axis), held against the definition of a
//! rolling-ball fillet point by point.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{SCALES, boxed, mass, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean, contains_points};
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};

fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let (i, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let d = (0..=32)
                .map(|j| {
                    let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0;
                    e.curve.eval(t).distance(p)
                })
                .fold(f64::INFINITY, f64::min);
            (i, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(i as u32)
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

/// The largest sine of the angle between the normals of a fillet face
/// and a kept face it borders, at points along their edges (the cuts at
/// the ends are creases by design and are left out: they border faces
/// across the edge, which are kept faces too, so only the borders along
/// the fillet's length are looked at: edges whose two faces are a fillet
/// and a face it is tangent to, told by the normals agreeing to 1e-3).
fn crease(body: &Body, faces: &[FilletFace]) -> f64 {
    let mut worst = 0.0_f64;
    let mut seams = 0;
    for (index, edge) in body.edges.iter().enumerate() {
        let users = faces_of(body, EdgeId(index as u32));
        let [a, b] = users[..] else { continue };
        let fillet = |f: FaceId| matches!(faces[f.0 as usize], FilletFace::Fillet(_));
        if fillet(a) == fillet(b) {
            continue;
        }
        let mut here = 0.0_f64;
        for i in 0..17 {
            let s = (f64::from(i) + 0.5) / 17.0;
            let p = edge
                .curve
                .eval(edge.range.0 + s * (edge.range.1 - edge.range.0));
            let (n, m) = (normal_at(body, a, p), normal_at(body, b, p));
            here = here.max(n.cross(m).length());
        }
        if here < 1e-3 {
            seams += 1;
            worst = worst.max(here);
        }
    }
    assert_eq!(seams, 2, "the two borders of the fillet");
    worst
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

/// The nearest point to `p` of the solid cone about z with radius `a` at
/// z = 0 narrowing to its apex at z = 3a (slope 1 : 3, as the cone of the
/// test), the radius moved in by `inset` across its side.
fn onto_cone(p: Vec3, a: f64) -> Vec3 {
    let rho = p.x.phypot(p.y);
    let (cx, cy) = if rho > 0.0 {
        (p.x / rho, p.y / rho)
    } else {
        (1.0, 0.0)
    };
    // In the half-plane (ρ, z): inside when ρ ≤ a − z / 3.
    if rho <= a - p.z / 3.0 {
        return p;
    }
    // Onto the line from (a, 0) along (−1, 3) / √10.
    let d = [-1.0 / 10f64.sqrt(), 3.0 / 10f64.sqrt()];
    let t = (rho - a) * d[0] + p.z * d[1];
    let apex = 3.0 * a * 10f64.sqrt() / 3.0;
    let t = t.min(apex);
    let (r, z) = (a + d[0] * t, d[1] * t);
    let r = r.max(0.0);
    Vec3::new(cx * r, cy * r, z)
}

/// The distance from `p` to the set of centres of balls of radius `r`
/// that lie behind both the plane y = 0 and the side of the cone (radius
/// `k` at z = 0, apex at z = 3k): the half-space y ≥ r and the cone moved
/// in by r, both convex, so their intersection is found by Dykstra's
/// alternating projections.
fn to_centres(p: Vec3, r: f64, k: f64) -> f64 {
    // The cone moved in by r across its side: its radius at z = 0 shrinks
    // by r / cos α, with cos α = 3 / √10.
    let a = k - r * 10f64.sqrt() / 3.0;
    let (mut x, mut pp, mut qq) = (p, Vec3::ZERO, Vec3::ZERO);
    for _ in 0..600 {
        let y0 = x + pp;
        let y = Vec3::new(y0.x, y0.y.max(r), y0.z);
        pp = y0 - y;
        let x0 = y + qq;
        x = onto_cone(x0, a);
        qq = x0 - x;
    }
    p.distance(x)
}

/// The rolling-ball fillet of radius r along the edge between the flat
/// side and the cone, by its definition: a point of the body is kept when
/// a ball of the radius behind both faces holds it. Every sample point of
/// a grid around the edge (and past both its ends) is classified by the
/// boolean's own ray casting in the result and held against that, except
/// within a margin of where the answer changes.
#[test]
fn a_straight_edge_between_a_plane_and_a_cone_by_its_definition() {
    let r = 0.15;
    for k in SCALES {
        let tol = tolerance(k);
        let body = half_frustum(k);
        let edge = edge_near(&body, Vec3::new(5.0 / 6.0 * k, 0.0, 0.5 * k));
        let made = fillet(
            &body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap();
        assert_eq!(check(&made.body, &tol), []);
        let n = 12;
        let mut points = Vec::new();
        let mut expected = Vec::new();
        let margin = 1e-4;
        for i in 0..n {
            for j in 0..n {
                for l in 0..n {
                    let f =
                        |m: usize, lo: f64, hi: f64| lo + (hi - lo) * (m as f64 + 0.37) / n as f64;
                    let p = Vec3::new(f(i, 0.4, 1.0), f(j, 0.0, 0.35), f(l, -0.05, 1.05));
                    let side = (1.0 - p.z / 3.0 - p.x.phypot(p.y)) * 3.0 / 10f64.sqrt();
                    let in_body = p.y > 0.0 && side > 0.0 && p.z > 0.0 && p.z < 1.0;
                    let near_body = [p.y, side, p.z, 1.0 - p.z].iter().any(|d| d.abs() < margin);
                    let d = to_centres(p, r, 1.0);
                    if near_body || (d - r).abs() < margin {
                        continue;
                    }
                    points.push(p * k);
                    expected.push(in_body && d <= r);
                }
            }
        }
        let found = contains_points(&made.body, &points, &tol).unwrap();
        let mut removed = 0;
        let mut undecided = 0;
        for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
            match got {
                Some(got) => assert_eq!(got, want, "at {p:?}, scale {k}"),
                None => undecided += 1,
            }
            // Points of the body the fillet took away.
            let q = *p * (1.0 / k);
            let side = (1.0 - q.z / 3.0 - q.x.phypot(q.y)) * 3.0 / 10f64.sqrt();
            if q.y > 0.0 && side > 0.0 && q.z > 0.0 && q.z < 1.0 && !want {
                removed += 1;
            }
        }
        assert!(undecided * 100 < points.len(), "{undecided} undecided");
        assert!(
            removed > 20,
            "only {removed} points in what the fillet takes away"
        );
    }
}

#[test]
fn a_straight_edge_between_a_plane_and_a_cone() {
    for k in SCALES {
        let tol = tolerance(k);
        let body = half_frustum(k);
        let edge = edge_near(&body, Vec3::new(5.0 / 6.0 * k, 0.0, 0.5 * k));
        let r = 0.15;
        let made = match fillet(
            &body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        ) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let worst = crease(&made.body, &made.faces);
        println!(
            "scale {k}: volume {} -> {}, largest crease {worst:e}",
            mass(&body).volume,
            mass(&made.body).volume
        );
        // Tangent to the angle the tolerance makes over the radius.
        assert!(worst <= tol.linear / (r * k), "a crease of {worst}");
    }
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
    /// The nearest point's parameter, by a fine search and Newton.
    fn nearest(&self, p: [f64; 2]) -> f64 {
        let mut best = (0.0, f64::INFINITY);
        for i in 0..=2000 {
            let t = f64::from(i) / 2000.0;
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
        0.25 * (b - a) * sum
    }
}

/// A blade section of two cubic Béziers that meet at both ends (the root
/// section of the turbofan's fan blade), extruded: the edges where they
/// meet are straight edges between two spline walls. Rounding one of them
/// is the 2D fillet of the two curves swept along it, worked out here on
/// its own (the ball's centre the radius in from the first curve, moved
/// along it until it is the radius from the second; the area by Green's
/// theorem over the curves and the arc).
#[test]
fn a_straight_edge_between_two_spline_walls() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
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
    let spline = |id: u64, b: &Bez| SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: b.0.to_vec(),
        }),
    };
    let height = 5.0;
    let r = 0.3;
    // The ball at the edge where the suction side starts (t = 0) and the
    // pressure side ends (t = 1): its centre is r in from the suction side
    // at the parameter where it is r from the pressure side too.
    let inward = |t: f64| {
        let d = suction.d(t);
        let len = d[0].phypot(d[1]);
        // The section runs clockwise suction then pressure: in is to the
        // right of the suction side.
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
    // The gap is negative right at the corner and positive further in.
    let (mut lo, mut hi) = (1e-4, 0.5);
    assert!(gap(lo) < 0.0 && gap(hi) > 0.0);
    for _ in 0..200 {
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
    // The region cut off: along the suction side from the corner to q1,
    // round the ball back to q2, along the pressure side to the corner.
    let a1 = (q1[1] - m[1]).patan2(q1[0] - m[0]);
    let mut a2 = (q2[1] - m[1]).patan2(q2[0] - m[0]);
    // The short way round: the arc that faces the corner spans less than
    // half a turn.
    let corner = suction.at(0.0);
    let wrap = |a: f64| {
        (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
    a2 = a1 + wrap(a2 - a1);
    let arc =
        r * r * (a2 - a1) + r * (m[0] * (a2.psin() - a1.psin()) - m[1] * (a2.pcos() - a1.pcos()));
    let removed = (suction.green(0.0, t1) + 0.5 * arc + pressure.green(t2, 1.0)).abs();
    for k in SCALES {
        let tol = tolerance(k);
        let curves: Vec<SketchCurve> = [spline(1, &suction), spline(2, &pressure)]
            .into_iter()
            .map(|c| {
                let Curve2::Spline(s) = c.curve else {
                    panic!("a spline")
                };
                SketchCurve {
                    id: c.id,
                    curve: Curve2::Spline(Spline2 {
                        control: s.control.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
                        ..s
                    }),
                }
            })
            .collect();
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let body = extrude(&profiles, &Frame::WORLD, 0.0, height * k, &tol)
            .unwrap()
            .body;
        let edge = edge_near(
            &body,
            Vec3::new(corner[0] * k, corner[1] * k, 0.5 * height * k),
        );
        let made = match fillet(
            &body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        ) {
            Ok(m) => m,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &tol), []);
        let before = mass(&body).volume;
        let after = mass(&made.body).volume;
        let expected = removed * height * k * k * k;
        println!("scale {k}: removed {} expected {expected}", before - after);
        assert!(
            ((before - after) - expected).abs() <= 1e-6 * expected,
            "removed {} instead of {expected}",
            before - after
        );
        let worst = crease(&made.body, &made.faces);
        assert!(worst <= tol.linear / (r * k), "a crease of {worst}");
    }
}

/// A chamfer along the same edge: its face is ruled between the two lines
/// on the faces the distances away from the edge (straight distances in
/// the plane across it), so every point of its borders on the plane and
/// on the cone is that far from the edge's line.
#[test]
fn a_straight_edge_between_a_plane_and_a_cone_chamfered() {
    use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
    let (d1, d2) = (0.2, 0.1);
    for k in SCALES {
        let tol = tolerance(k);
        let body = half_frustum(k);
        let edge = edge_near(&body, Vec3::new(5.0 / 6.0 * k, 0.0, 0.5 * k));
        let plane = faces_of(&body, edge)
            .into_iter()
            .find(|f| {
                matches!(
                    body.faces[f.0 as usize].surface,
                    craft3d_geom::Surface::Plane(_)
                )
            })
            .unwrap();
        let line = body.edges[edge.0 as usize].clone();
        let (a, b) = (line.curve.eval(line.range.0), line.curve.eval(line.range.1));
        let along = (b - a).normalized().unwrap();
        let made = chamfer(
            &body,
            &[ChamferEdge {
                edge,
                first: plane,
                size: ChamferSize::Distances {
                    first: d1 * k,
                    second: d2 * k,
                },
            }],
            &tol,
        )
        .unwrap();
        assert_eq!(check(&made.body, &tol), []);
        let face = made
            .faces
            .iter()
            .position(|f| matches!(f, ChamferFace::Chamfer(_)))
            .unwrap();
        let mut borders = 0;
        for (index, e) in made.body.edges.iter().enumerate() {
            let users = faces_of(&made.body, EdgeId(index as u32));
            if !users.contains(&FaceId(face as u32)) {
                continue;
            }
            // The borders run along the edge; the cuts at the ends across.
            let (p, q) = (e.curve.eval(e.range.0), e.curve.eval(e.range.1));
            if (q - p).normalized().unwrap().dot(along).abs() < 0.9 {
                continue;
            }
            let other = users.into_iter().find(|f| f.0 as usize != face).unwrap();
            let want = if other == plane { d1 } else { d2 };
            borders += 1;
            for i in 0..=16 {
                let x = e
                    .curve
                    .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(i) / 16.0);
                let off = x - a;
                let d = (off - along * off.dot(along)).length();
                assert!(
                    (d - want * k).abs() <= tol.linear,
                    "{d} is not {}",
                    want * k
                );
            }
        }
        assert_eq!(borders, 2);
        assert!(mass(&made.body).volume < mass(&body).volume);
    }
}

/// The plane of the slanted cut of the rod below: through `o`, its
/// outward normal `n`.
fn slanted() -> (Vec3, Vec3) {
    (
        Vec3::new(-3.0, -3.0, 1.5),
        Vec3::new(0.3, 0.0, 1.0).normalized().unwrap(),
    )
}

/// A rod of radius 1 from z = 0 to 3, cut by a slanted plane: the rim of
/// the cut is a whole ellipse between the plane and the cylinder, a
/// closed edge with one vertex.
fn slanted_rod(k: f64) -> Body {
    let tol = tolerance(k);
    let rod = primitives::cylinder(&Frame::WORLD, k, 3.0 * k).unwrap();
    let (o, n) = slanted();
    let tilted = Frame::new(o * k, n, Vec3::X).unwrap();
    let cutter = primitives::block(&tilted, 6.0 * k, 6.0 * k, 6.0 * k).unwrap();
    boolean(&rod, &cutter, BooleanOp::Subtract, &tol)
        .unwrap()
        .body
}

/// The distance from `p` to the centres of the balls of radius `r` behind
/// the slanted plane and inside the rod: a half-space and a cylinder moved
/// in by r, Dykstra again.
fn to_rim_centres(p: Vec3, r: f64) -> f64 {
    let (o, n) = slanted();
    let (mut x, mut pp, mut qq) = (p, Vec3::ZERO, Vec3::ZERO);
    for _ in 0..600 {
        let y0 = x + pp;
        let over = n.dot(y0 - o) + r;
        let y = if over > 0.0 { y0 - n * over } else { y0 };
        pp = y0 - y;
        let x0 = y + qq;
        let rho = x0.x.phypot(x0.y);
        x = if rho > 1.0 - r {
            Vec3::new(x0.x * (1.0 - r) / rho, x0.y * (1.0 - r) / rho, x0.z)
        } else {
            x0
        };
        qq = x0 - x;
    }
    p.distance(x)
}

/// The rim of the slanted cut rounded: a closed freeform edge, its fillet
/// a ring of spline surface, held point by point against the definition
/// as the cone's edge is.
#[test]
fn the_rim_of_a_slanted_cut_of_a_rod() {
    let r = 0.2;
    let (o, n) = slanted();
    for k in SCALES {
        let tol = tolerance(k);
        let body = slanted_rod(k);
        let rim = (0..body.edges.len())
            .find(|i| body.edges[*i].curve.kind() == "ellipse")
            .unwrap();
        let made = fillet(
            &body,
            &[FilletEdge {
                edge: EdgeId(rim as u32),
                radius: r * k,
            }],
            &tol,
        )
        .unwrap();
        assert_eq!(check(&made.body, &tol), []);
        assert_eq!(made.body.faces.len(), 4);
        let worst = crease(&made.body, &made.faces);
        assert!(worst <= tol.linear / (r * k), "a crease of {worst}");
        let n_grid = 12;
        let margin = 1e-4;
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for i in 0..n_grid {
            for j in 0..n_grid {
                for l in 0..n_grid {
                    let f = |m: usize, lo: f64, hi: f64| {
                        lo + (hi - lo) * (m as f64 + 0.37) / n_grid as f64
                    };
                    let p = Vec3::new(f(i, -1.05, 1.05), f(j, -1.05, 1.05), f(l, 0.0, 1.8));
                    let side = 1.0 - p.x.phypot(p.y);
                    let below = -n.dot(p - o);
                    let in_body = side > 0.0 && below > 0.0 && p.z > 0.0;
                    let near_body = [side, below, p.z].iter().any(|d| d.abs() < margin);
                    let d = to_rim_centres(p, r);
                    if near_body || (d - r).abs() < margin {
                        continue;
                    }
                    points.push(p * k);
                    expected.push(in_body && d <= r);
                }
            }
        }
        let found = contains_points(&made.body, &points, &tol).unwrap();
        let mut undecided = 0;
        for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
            match got {
                Some(got) => assert_eq!(got, want, "at {p:?}, scale {k}"),
                None => undecided += 1,
            }
        }
        assert!(undecided * 100 < points.len(), "{undecided} undecided");
        assert!(mass(&made.body).volume < mass(&body).volume);
    }
}
