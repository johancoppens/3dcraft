//! A blend whose border is carried across a smooth edge of its face
//! (`kernel::blend::rolling::cross`): a prism whose top's back edge was
//! rounded first, so the top goes on smoothly into the round across the
//! round's tangent line, and whose front top edge is then rounded with a
//! radius that reaches past that line. Where the back edge runs at a
//! slant the border crosses the tangent line part of the way along (from
//! one end the ball rolls against the round); where it is parallel and
//! nearer the front than the radius the top lies wholly in what the blend
//! takes and goes. Held against the definition of a rolling-ball fillet
//! (a point is kept exactly when it is in the body and a ball of the
//! radius behind the front and the rounded top holds it: the distance to
//! the set of such centres, an intersection of two convex sets, by
//! Dykstra's projections), every mesh point of the fillet on it, at three
//! scales in a tilted frame.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, assert_close, mass, tolerance};
use common::polygon;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::contains_points;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, check};

fn frame(k: f64) -> Frame {
    Frame::new(
        Vec3::new(0.3, -0.2, 0.5) * k,
        Vec3::new(0.2, 0.3, 1.0),
        Vec3::new(1.0, 0.0, -0.2),
    )
    .unwrap()
}

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

/// The prism: in its own coordinates the front is y = 0, the top z = h,
/// the ends x = 0 and x = l, the back the plane through (l, a) and
/// (0, b), upright; its top back edge rounded by `big`.
struct Part {
    l: f64,
    a: f64,
    b: f64,
    h: f64,
    big: f64,
}

impl Part {
    /// The back's outward unit normal in the plane, and its offset: a
    /// point is in front of the back where `n · p < c`.
    fn back(&self) -> ([f64; 2], f64) {
        let (dx, dy) = (self.l, self.a - self.b);
        let len = dx.hypot(dy);
        let n = [-dy / len, dx / len];
        (n, n[1] * self.b)
    }

    /// The edge of the wedge the round is made of, moved in by `big`
    /// from the top and the back: the round is the points within `big`
    /// of the wedge behind it.
    fn wedge(&self, p: [f64; 3]) -> [f64; 3] {
        let (n, c) = self.back();
        let over = n[0] * p[0] + n[1] * p[1] - (c - self.big);
        let (x, y) = if over > 0.0 {
            (p[0] - n[0] * over, p[1] - n[1] * over)
        } else {
            (p[0], p[1])
        };
        [x, y, p[2].min(self.h - self.big)]
    }

    /// How far `p` is from the rounded top and back's wedge.
    fn off_wedge(&self, p: [f64; 3]) -> f64 {
        let q = self.wedge(p);
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    }

    fn body(&self, f: &Frame, k: f64, tol: &Tolerance) -> Body {
        let pts = [[0.0, 0.0], [self.l, 0.0], [self.l, self.a], [0.0, self.b]];
        let curves = polygon(1, 1, &pts.map(|p| [p[0] * k, p[1] * k]));
        let profiles = detect_profiles(&curves, tol).unwrap();
        let prism = extrude(&profiles, f, 0.0, self.h * k, tol).unwrap().body;
        let back = edge_near(
            &prism,
            f.point(0.5 * self.l * k, 0.5 * (self.a + self.b) * k, self.h * k),
        );
        let made = fillet(
            &prism,
            &[FilletEdge {
                edge: back,
                radius: self.big * k,
            }],
            tol,
        )
        .unwrap();
        assert_eq!(check(&made.body, tol), []);
        made.body
    }

    /// Projection onto the centres of the balls of radius `r` behind the
    /// front and the rounded top: y at least r, and within big - r of
    /// the wedge (Dykstra's alternating projections).
    fn to_centres(&self, p: [f64; 3], r: f64) -> f64 {
        let rho = self.big - r;
        let onto_front = |q: [f64; 3]| [q[0], q[1].max(r), q[2]];
        let onto_round = |q: [f64; 3]| {
            let w = self.wedge(q);
            let d = ((q[0] - w[0]).powi(2) + (q[1] - w[1]).powi(2) + (q[2] - w[2]).powi(2)).sqrt();
            if d <= rho {
                q
            } else {
                let s = rho / d;
                [
                    w[0] + (q[0] - w[0]) * s,
                    w[1] + (q[1] - w[1]) * s,
                    w[2] + (q[2] - w[2]) * s,
                ]
            }
        };
        let (mut x, mut pa, mut qa) = (p, [0.0; 3], [0.0; 3]);
        for _ in 0..4000 {
            let y = onto_front([x[0] + pa[0], x[1] + pa[1], x[2] + pa[2]]);
            pa = [
                x[0] + pa[0] - y[0],
                x[1] + pa[1] - y[1],
                x[2] + pa[2] - y[2],
            ];
            let z = onto_round([y[0] + qa[0], y[1] + qa[1], y[2] + qa[2]]);
            qa = [
                y[0] + qa[0] - z[0],
                y[1] + qa[1] - z[1],
                y[2] + qa[2] - z[2],
            ];
            let moved =
                ((z[0] - x[0]).powi(2) + (z[1] - x[1]).powi(2) + (z[2] - x[2]).powi(2)).sqrt();
            x = z;
            if moved < 1e-15 {
                break;
            }
        }
        ((p[0] - x[0]).powi(2) + (p[1] - x[1]).powi(2) + (p[2] - x[2]).powi(2)).sqrt()
    }
}

fn grid(lo: [f64; 3], hi: [f64; 3], n: usize) -> Vec<[f64; 3]> {
    let f = |m: usize, a: f64, b: f64| a + (b - a) * (m as f64 + 0.37) / n as f64;
    let mut out = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for l in 0..n {
                out.push([f(i, lo[0], hi[0]), f(j, lo[1], hi[1]), f(l, lo[2], hi[2])]);
            }
        }
    }
    out
}

/// Every point of the grid in or out as the definition says, every mesh
/// point of the fillet faces on the definition's surface within the
/// tolerance; with how many points the fillet moved.
fn held(
    part: &Part,
    r: f64,
    k: f64,
    tol: &Tolerance,
    made: &craft3d_kernel::fillet::FilletResult,
    before: &Body,
    what: &str,
) {
    let f = frame(k);
    assert_eq!(check(&made.body, tol), [], "{what}");
    let deep = body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear)).unwrap();
    assert_eq!(deep.certain_crossings, 0, "{what}: {deep:?}");
    let margin = 2e-4;
    let (n, c) = part.back();
    let want = |p: [f64; 3]| -> Option<bool> {
        let back = n[0] * p[0] + n[1] * p[1] - c;
        let round = part.off_wedge(p) - part.big;
        let d = part.to_centres(p, r);
        let values = [p[0], p[0] - part.l, p[1], p[2], back, round, d - r];
        if values.iter().any(|v| v.abs() < margin) {
            return None;
        }
        let inside =
            p[0] > 0.0 && p[0] < part.l && p[1] > 0.0 && p[2] > 0.0 && back < 0.0 && round < 0.0;
        Some(inside && d <= r)
    };
    let (mut points, mut expected) = (Vec::new(), Vec::new());
    for p in grid(
        [-0.05, -0.05, part.h - 0.6],
        [part.l + 0.05, 0.7, part.h + 0.05],
        16,
    ) {
        if let Some(w) = want(p) {
            points.push(f.point(p[0] * k, p[1] * k, p[2] * k));
            expected.push(w);
        }
    }
    let found = contains_points(&made.body, &points, tol).unwrap();
    let was = contains_points(before, &points, tol).unwrap();
    let (mut undecided, mut changed) = (0, 0);
    for (((p, w), got), old) in points.iter().zip(&expected).zip(&found).zip(&was) {
        match got {
            Some(got) => assert_eq!(got, w, "{what}: at {p:?}"),
            None => undecided += 1,
        }
        if *old != Some(*w) {
            changed += 1;
        }
    }
    assert!(
        undecided * 100 < points.len(),
        "{what}: {undecided} undecided"
    );
    assert!(changed > 40, "{what}: only {changed} points changed sides");
    let params = TessParams {
        chordal_tolerance: 1e-3 * k,
        ..TessParams::default()
    };
    let mesh = tessellate_body(&made.body, &params).unwrap();
    let (mut worst, mut seen) = (0.0_f64, 0);
    for (t, face) in mesh.face_ids.iter().enumerate() {
        if !matches!(made.faces[*face as usize], FilletFace::Fillet(_)) {
            continue;
        }
        for p in mesh.corners(t).unwrap() {
            let (x, y, z) = f.local(p);
            let g = part.to_centres([x / k, y / k, z / k], r) - r;
            worst = worst.max(g.abs());
            seen += 1;
        }
    }
    println!(
        "{what}: {} points, {changed} changed, {undecided} undecided; {seen} mesh points off the definition by {:e} of the tolerance",
        points.len(),
        worst * k / tol.linear
    );
    assert!(seen > 50, "{what}: {seen} mesh points");
    assert!(worst * k <= tol.linear, "{what}: {} off", worst * k);
}

/// The back runs at a slant: the round's tangent line on the top comes
/// within the radius of the front edge towards the right end, so the
/// border crosses it part of the way along, and from there on the ball
/// rolls against the round; at the right end the fillet is cut where the
/// round meets the end face.
#[test]
fn a_border_crosses_the_tangent_line_of_an_earlier_round() {
    let part = Part {
        l: 1.0,
        a: 0.75,
        b: 1.25,
        h: 1.0,
        big: 0.6,
    };
    let r = 0.3;
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        let body = part.body(&f, k, &tol);
        let front = edge_near(&body, f.point(0.5 * part.l * k, 0.0, part.h * k));
        let made = fillet(
            &body,
            &[FilletEdge {
                edge: front,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        held(
            &part,
            r,
            k,
            &tol,
            &made,
            &body,
            &format!("slanted, scale {k}"),
        );
        // One fillet face, between the front, the top and the round.
        let fillets = made
            .faces
            .iter()
            .filter(|f| matches!(f, FilletFace::Fillet(_)))
            .count();
        assert_eq!(fillets, 1);
        assert_eq!(made.body.faces.len(), body.faces.len() + 1);
    }
}

/// The back is parallel to the front, the round's tangent line nearer
/// the front than the radius all along: the top lies wholly in what the
/// fillet takes and goes, and the fillet rolls against the front and the
/// round, an exact cylinder (the same in every plane across the edge).
/// Its section's area in closed form gives the volume.
#[test]
fn a_face_wholly_inside_what_a_fillet_takes_goes() {
    let part = Part {
        l: 1.0,
        a: 0.75,
        b: 0.75,
        h: 1.0,
        big: 0.6,
    };
    let r = 0.3;
    // In the section: the round's centre at (y, z) = (b - big, h - big),
    // the ball's at y = r, big - r from it.
    let (cy, cz) = (part.b - part.big, part.h - part.big);
    let mz = cz + ((part.big - r).powi(2) - (r - cy).powi(2)).sqrt();
    // What the fillet takes from the section (y, z): the region bounded
    // by the front from the ball's foot (0, mz) up to the edge (0, h),
    // the top to where the round begins (cy, h), the round down to where
    // the ball touches it, and the ball's arc over its top back to its
    // foot; by Green's theorem (half the integral of y dz - z dy).
    let (dy, dz) = (r - cy, mz - cz);
    let len = dy.hypot(dz);
    let touch = [cy + dy / len * part.big, cz + dz / len * part.big];
    let angle = |c: [f64; 2], p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
    let line = |p: [f64; 2], q: [f64; 2]| p[0] * q[1] - p[1] * q[0];
    let arc = |c: [f64; 2], rad: f64, a: f64, b: f64| {
        rad * rad * (b - a) + rad * (c[0] * (b.sin() - a.sin()) - c[1] * (b.cos() - a.cos()))
    };
    let h = part.h;
    let twice = line([0.0, mz], [0.0, h])
        + line([0.0, h], [cy, h])
        + arc(
            [cy, cz],
            part.big,
            std::f64::consts::FRAC_PI_2,
            angle([cy, cz], touch),
        )
        + arc([r, mz], r, angle([r, mz], touch), std::f64::consts::PI);
    let taken = (0.5 * twice).abs();
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        let body = part.body(&f, k, &tol);
        let front = edge_near(&body, f.point(0.5 * part.l * k, 0.0, part.h * k));
        let made = fillet(
            &body,
            &[FilletEdge {
                edge: front,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        held(
            &part,
            r,
            k,
            &tol,
            &made,
            &body,
            &format!("parallel, scale {k}"),
        );
        // The top is gone: one face fewer, then the fillet's.
        assert_eq!(made.body.faces.len(), body.faces.len());
        let cylinders = made
            .body
            .faces
            .iter()
            .zip(&made.faces)
            .filter(|(face, from)| {
                matches!(from, FilletFace::Fillet(_))
                    && matches!(face.surface, Surface::Cylinder(_))
            })
            .count();
        assert_eq!(cylinders, 1, "the fillet is an exact cylinder");
        let lost = mass(&body).volume - mass(&made.body).volume;
        assert_close(lost, taken * part.l * k * k * k, "volume taken");
    }
}

/// The same edge chamfered across the tangent line: the chamfer's foot on
/// the top side is the point of the top or the round, in the plane
/// across the edge, the distance from the edge.
#[test]
fn a_chamfer_border_crosses_the_tangent_line_of_an_earlier_round() {
    let part = Part {
        l: 1.0,
        a: 0.75,
        b: 1.25,
        h: 1.0,
        big: 0.6,
    };
    let d = 0.3;
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        let body = part.body(&f, k, &tol);
        let front = edge_near(&body, f.point(0.5 * part.l * k, 0.0, part.h * k));
        // The top as the second face and as the first: the border carried
        // across on either side of the construction.
        for first in [[0.5, 0.0, 0.5], [0.5, 0.05, part.h]] {
            let made = chamfer(
                &body,
                &[ChamferEdge {
                    edge: front,
                    first: body_face_at(&body, &f, k, first),
                    size: ChamferSize::Distances {
                        first: d * k,
                        second: d * k,
                    },
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("scale {k}: {e}"));
            assert_eq!(check(&made.body, &tol), [], "scale {k}");
            // In each plane across the edge (x fixed) the chamfer is the
            // chord from (0, h - d) on the front to the point of the top
            // sheet d from the edge's point (0, h): every point of the grid
            // in the body on the edge's side of that chord is gone.
            let (n, c) = part.back();
            let sheet_foot = |x: f64| -> [f64; 2] {
                // Along the sheet's section from the edge, by bisection on the
                // distance from (0, h): the top at z = h up to the round, then
                // the round (points within big of the wedge, at its border).
                let at = |t: f64| -> [f64; 2] {
                    // t: y along the top, continued over the round by the
                    // point of the round straight below (the section's border).
                    let y = t;
                    let mut lo = part.h - 2.0 * part.big;
                    let mut hi = part.h;
                    for _ in 0..100 {
                        let z = 0.5 * (lo + hi);
                        if part.off_wedge([x, y, z]) <= part.big {
                            lo = z
                        } else {
                            hi = z
                        }
                    }
                    [y, 0.5 * (lo + hi)]
                };
                let (mut lo, mut hi) = (0.0, part.b);
                for _ in 0..100 {
                    let t = 0.5 * (lo + hi);
                    let p = at(t);
                    if p[0].hypot(part.h - p[1]) < d {
                        lo = t
                    } else {
                        hi = t
                    }
                }
                at(0.5 * (lo + hi))
            };
            let margin = 2e-4;
            let mut points = Vec::new();
            let mut expected = Vec::new();
            for p in grid(
                [0.02, -0.05, part.h - 0.5],
                [part.l - 0.02, 0.6, part.h + 0.05],
                14,
            ) {
                let foot = sheet_foot(p[0]);
                let q1 = [0.0, part.h - d];
                // The chord's normal towards the edge (0, h).
                let (ty, tz) = (foot[0] - q1[0], foot[1] - q1[1]);
                let mut nn = [tz, -ty];
                if nn[0] * (0.0 - q1[0]) + nn[1] * (part.h - q1[1]) < 0.0 {
                    nn = [-nn[0], -nn[1]];
                }
                let len = nn[0].hypot(nn[1]);
                let side = (nn[0] * (p[1] - q1[0]) + nn[1] * (p[2] - q1[1])) / len;
                let back = n[0] * p[0] + n[1] * p[1] - c;
                let round = part.off_wedge(p) - part.big;
                if [p[1], p[2] - part.h, back, round, side]
                    .iter()
                    .any(|v| v.abs() < margin)
                {
                    continue;
                }
                let inside = p[1] > 0.0 && back < 0.0 && round < 0.0;
                points.push(f.point(p[0] * k, p[1] * k, p[2] * k));
                expected.push(inside && side < 0.0);
            }
            let found = contains_points(&made.body, &points, &tol).unwrap();
            let mut changed = 0;
            let was = contains_points(&body, &points, &tol).unwrap();
            for (((p, w), got), old) in points.iter().zip(&expected).zip(&found).zip(&was) {
                if let Some(got) = got {
                    assert_eq!(got, w, "scale {k}: at {p:?}");
                }
                if *old != Some(*w) {
                    changed += 1;
                }
            }
            assert!(changed > 40, "scale {k}: {changed} changed");
        }
    }
}

/// The face of `body` nearest the point (in the part's units).
fn body_face_at(body: &Body, f: &Frame, k: f64, p: [f64; 3]) -> craft3d_topo::FaceId {
    let q = f.point(p[0] * k, p[1] * k, p[2] * k);
    let (i, _) = body
        .faces
        .iter()
        .enumerate()
        .filter_map(|(i, face)| {
            let (u, v) = face.surface.project(q).ok()?;
            Some((i, face.surface.eval(u, v).distance(q)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    craft3d_topo::FaceId(i as u32)
}

/// An inside corner: a wall standing at y = 0 on a floor whose far edge
/// (at y = w + big) is rounded by `big`, so the floor's flat strip between
/// the wall and the round's tangent line is narrower than the fillet's
/// radius. The root fillet between wall and floor fills that strip whole
/// (the floor goes) and rolls against the wall and the round: an exact
/// cylinder, its section's area in closed form.
#[test]
fn an_inside_fillet_fills_a_floor_strip_narrower_than_its_radius() {
    let (w, big, r, h, l) = (0.15_f64, 0.4_f64, 0.3_f64, 1.0_f64, 1.0_f64);
    let far = w + big;
    // The ball is r from the wall, big + r from the round's axis at
    // (w, -big).
    let zc = -big + ((big + r).powi(2) - (r - w).powi(2)).sqrt();
    let (dy, dz) = (r - w, zc + big);
    let len = dy.hypot(dz);
    let touch = [w + dy / len * big, -big + dz / len * big];
    let angle = |c: [f64; 2], p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
    let line = |p: [f64; 2], q: [f64; 2]| p[0] * q[1] - p[1] * q[0];
    let arc = |c: [f64; 2], rad: f64, a: f64, b: f64| {
        rad * rad * (b - a) + rad * (c[0] * (b.sin() - a.sin()) - c[1] * (b.cos() - a.cos()))
    };
    // Up the wall to the ball's foot, round the ball's lower side to the
    // round, up the round to the floor, back along the floor.
    let mut to_touch = angle([r, zc], touch);
    while to_touch < std::f64::consts::PI {
        to_touch += 2.0 * std::f64::consts::PI;
    }
    let twice = line([0.0, 0.0], [0.0, zc])
        + arc([r, zc], r, std::f64::consts::PI, to_touch)
        + arc(
            [w, -big],
            big,
            angle([w, -big], touch),
            std::f64::consts::FRAC_PI_2,
        )
        + line([w, 0.0], [0.0, 0.0]);
    let added = (0.5 * twice).abs();
    assert!(added > 0.0 && added < r * r);
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        // The section in the plane of y and z, swept along x.
        let across = Frame::new(f.origin(), f.x(), f.y()).unwrap();
        let pts = [
            [-0.5, -1.0],
            [far, -1.0],
            [far, 0.0],
            [0.0, 0.0],
            [0.0, h],
            [-0.5, h],
        ];
        let curves = polygon(1, 1, &pts.map(|p| [p[0] * k, p[1] * k]));
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let body = extrude(&profiles, &across, 0.0, l * k, &tol).unwrap().body;
        let rim = edge_near(&body, f.point(0.5 * l * k, far * k, 0.0));
        let rounded = fillet(
            &body,
            &[FilletEdge {
                edge: rim,
                radius: big * k,
            }],
            &tol,
        )
        .unwrap()
        .body;
        let root = edge_near(&rounded, f.point(0.5 * l * k, 0.0, 0.0));
        let made = fillet(
            &rounded,
            &[FilletEdge {
                edge: root,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert_eq!(check(&made.body, &tol), [], "scale {k}");
        let deep = body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear)).unwrap();
        assert_eq!(deep.certain_crossings, 0, "scale {k}");
        // The floor's strip is gone, the fillet is a cylinder.
        assert_eq!(made.body.faces.len(), rounded.faces.len());
        let cylinders = made
            .body
            .faces
            .iter()
            .zip(&made.faces)
            .filter(|(face, from)| {
                matches!(from, FilletFace::Fillet(_))
                    && matches!(face.surface, Surface::Cylinder(_))
            })
            .count();
        assert_eq!(cylinders, 1, "scale {k}");
        let gained = mass(&made.body).volume - mass(&rounded).volume;
        assert_close(gained, added * l * k * k * k, "volume added");
    }
}

/// A shallow hole in the round, inside the strip the fillet takes past
/// the crossing: the border carried onto the round would leave the
/// hole's loop outside its face. Refused as reaching that face.
#[test]
fn a_hole_in_the_face_beyond_a_crossing_is_not_left_behind() {
    let part = Part {
        l: 1.0,
        a: 0.75,
        b: 1.25,
        h: 1.0,
        big: 0.6,
    };
    let r = 0.3;
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        let body = part.body(&f, k, &tol);
        // At x = 0.9 the tangent line lies at y = g; a point of the round
        // a little past it, within the strip (y < r): drilled there
        // along the round's normal.
        let x = 0.9;
        let (n, c) = part.back();
        let g = (c - part.big - n[0] * x) / n[1];
        // Down the round from the tangent line, by an angle about its
        // axis (which runs along the back, big below the top).
        let phi = 0.06_f64 / part.big;
        let centre = [x, g, part.h - part.big];
        let out = [n[0] * phi.sin(), n[1] * phi.sin(), phi.cos()];
        let p = [
            centre[0] + out[0] * part.big,
            centre[1] + out[1] * part.big,
            centre[2] + out[2] * part.big,
        ];
        assert!(
            p[1] < r && p[1] > g,
            "the hole lies in the strip past the line"
        );
        let at = f.point(p[0] * k, p[1] * k, p[2] * k);
        let normal = f.vector(out[0], out[1], out[2]);
        let hole_frame = Frame::new(at - normal * (0.02 * k), normal, f.x()).unwrap();
        let drill = craft3d_kernel::primitives::cylinder(&hole_frame, 0.02 * k, 0.5 * k).unwrap();
        let drilled = craft3d_kernel::boolean::boolean(
            &body,
            &drill,
            craft3d_kernel::boolean::BooleanOp::Subtract,
            &tol,
        )
        .unwrap()
        .body;
        let front = edge_near(&drilled, f.point(0.5 * part.l * k, 0.0, part.h * k));
        let made = fillet(
            &drilled,
            &[FilletEdge {
                edge: front,
                radius: r * k,
            }],
            &tol,
        );
        assert!(made.is_err(), "scale {k}: built through the hole");
    }
}
