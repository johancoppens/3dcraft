//! The review's own oracle of a fillet whose radius is a law along its
//! edge (review-fade-law), written from the definition alone (DESIGN.md
//! §3.2, blends.md): in the plane across the edge at the length `a` along
//! it, the ball of radius `r(a)` that touches both faces; the face is the
//! arcs of those balls between their feet. Nothing of the construction is
//! used: the edge's length is a sum of chords (Richardson), the ball is
//! found by Newton's method on the signed distances to the two surfaces
//! (geom's projection for the foot, normals by central differences of the
//! surfaces' points), the arc that holds a point by the secant rule on the
//! side of the arc's plane it lies.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use super::boolean_common::tolerance;
use craft3d_geom::Curve3;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{
    Fade, FilletFace, FilletLawEdge, FilletResult, RadiusLaw, fillet_by_law,
};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId};
use std::f64::consts::PI;

/// The law as the design and blends.md give it, written here: one radius,
/// and per end of the edge `(to, over)`; `backs`: how far short of that
/// end a fade to nothing (`to` zero, a tip) ends.
#[derive(Clone, Copy, Debug)]
pub struct Law {
    pub radius: f64,
    pub fades: [Option<(f64, f64)>; 2],
    pub backs: [f64; 2],
}

impl Law {
    /// The radius at the length `a` from the edge's start, the edge being
    /// `length` long. Past an end (a fade to a radius) the continuation
    /// the branch documents now, which runs on one way:
    /// `to + A·(1 − exp(aτ + bτ²))`, `A` = −to falling, to − radius
    /// growing, `a = −3Δ/A`, `b = (6Δ/A − a²)/2`. Beyond a tip: nothing.
    pub fn r(&self, a: f64, length: f64) -> f64 {
        let mut r = self.radius;
        for ((fade, from_end), back) in self.fades.iter().zip([a, length - a]).zip(self.backs) {
            let Some((to, over)) = *fade else { continue };
            let tau = (from_end - back) / over;
            let change = self.radius - to;
            if tau < 0.0 {
                if to == 0.0 {
                    r = 0.0;
                } else if change != 0.0 {
                    let big = if change > 0.0 { -to } else { to - self.radius };
                    let ea = -3.0 * change / big;
                    let eb = 0.5 * (6.0 * change / big - ea * ea);
                    r = to + big * (1.0 - (ea * tau + eb * tau * tau).exp());
                } else {
                    r = to;
                }
            } else if tau < 1.0 {
                let m = 1.0 - tau;
                r = to + change * (1.0 - m * m * m);
            }
        }
        r
    }

    /// The old continuation (S1 as first reviewed, 3ceed7d2):
    /// `to + Δ·3τ/(1 + τ + dτ²)`.
    pub fn r_levelling(&self, a: f64, length: f64) -> f64 {
        let mut r = self.radius;
        for (fade, from_end) in self.fades.iter().zip([a, length - a]) {
            let Some((to, over)) = *fade else { continue };
            let tau = from_end / over;
            let change = self.radius - to;
            if tau < 0.0 {
                let reach = 0.5 * (6.0 * change.abs() / to + 1.0);
                let d = (reach * reach).max(4.0);
                r = to + change * 3.0 * tau / (1.0 + tau + d * tau * tau);
            } else if tau < 1.0 {
                let m = 1.0 - tau;
                r = to + change * (1.0 - m * m * m);
            }
        }
        r
    }
}

/// An edge by its length: a table of the curve's parameter against the
/// length from the edge's start (chords, improved by Richardson's step).
pub struct ByLength {
    pub curve: Curve3,
    pub t0: f64,
    pub t1: f64,
    pub ts: Vec<f64>,
    pub lengths: Vec<f64>,
}

impl ByLength {
    pub fn new(body: &Body, edge: EdgeId) -> Self {
        let e = body.edge(edge).unwrap();
        let (t0, t1) = e.range;
        // Past both ends too (a quarter of the range each way).
        let (lo, hi) = (t0 - 0.25 * (t1 - t0), t1 + 0.25 * (t1 - t0));
        let n = 1 << 14;
        let mut ts = Vec::with_capacity(n + 1);
        let mut lengths = Vec::with_capacity(n + 1);
        let mut coarse = 0.0;
        let mut fine = 0.0;
        for i in 0..=n {
            let t = lo + (hi - lo) * i as f64 / n as f64;
            if i > 0 {
                let a = lo + (hi - lo) * (i - 1) as f64 / n as f64;
                let m = 0.5 * (a + t);
                let (pa, pm, pt) = (e.curve.eval(a), e.curve.eval(m), e.curve.eval(t));
                coarse += pa.distance(pt);
                fine += pa.distance(pm) + pm.distance(pt);
            }
            ts.push(t);
            lengths.push(fine + (fine - coarse) / 3.0);
        }
        let mut made = ByLength {
            curve: e.curve.clone(),
            t0,
            t1,
            ts,
            lengths,
        };
        let zero = made.length_at(t0);
        for l in &mut made.lengths {
            *l -= zero;
        }
        made
    }

    fn length_at(&self, t: f64) -> f64 {
        let i = self
            .ts
            .partition_point(|x| *x <= t)
            .clamp(1, self.ts.len() - 1);
        let (a, b) = (self.ts[i - 1], self.ts[i]);
        let w = (t - a) / (b - a);
        self.lengths[i - 1] + w * (self.lengths[i] - self.lengths[i - 1])
    }

    pub fn length(&self) -> f64 {
        self.length_at(self.t1)
    }

    pub fn t_at(&self, a: f64) -> f64 {
        let i = self
            .lengths
            .partition_point(|x| *x <= a)
            .clamp(1, self.lengths.len() - 1);
        let (la, lb) = (self.lengths[i - 1], self.lengths[i]);
        let w = (a - la) / (lb - la);
        self.ts[i - 1] + w * (self.ts[i] - self.ts[i - 1])
    }

    /// The point at the length `a` and the unit tangent there.
    pub fn at(&self, a: f64) -> (Vec3, Vec3) {
        let t = self.t_at(a);
        let h = 1e-6 * (self.t1 - self.t0);
        let d = self.curve.eval(t + h) - self.curve.eval(t - h);
        (self.curve.eval(t), d.normalized().unwrap())
    }

    /// The length at the point of the table nearest `p`.
    pub fn nearest(&self, p: Vec3) -> f64 {
        let mut best = (0.0, f64::INFINITY);
        for (i, t) in self.ts.iter().enumerate().step_by(4) {
            let d = self.curve.eval(*t).distance(p);
            if d < best.1 {
                best = (self.lengths[i], d);
            }
        }
        best.0
    }
}

/// The outward unit normal of a face at its parameters, by central
/// differences of its surface's points (the sign by the surface's own
/// normal and the face's `reversed`).
pub fn normal(body: &Body, face: FaceId, uv: (f64, f64)) -> Vec3 {
    let f = body.face(face).unwrap();
    let s = &f.surface;
    let (u, v) = uv;
    // Steps that move the point by a millionth of its distance from the
    // origin: far from the origin a step in the parameters' own units
    // would be lost in the rounding of the coordinates.
    let h0u = 1e-4 * u.abs().max(1.0);
    let h0v = 1e-4 * v.abs().max(1.0);
    let rate_u = (s.eval(u + h0u, v) - s.eval(u - h0u, v)).length() / (2.0 * h0u);
    let rate_v = (s.eval(u, v + h0v) - s.eval(u, v - h0v)).length() / (2.0 * h0v);
    // The size of the coordinates, and at least what the parameters
    // themselves span (a point at the origin has no size of its own).
    let size = (s.eval(u, v).length() + 1e4 * (rate_u * h0u).max(rate_v * h0v)).max(1e-300);
    let step = |rate: f64| 1e-6 * size / rate.max(1e-300);
    let (hu, hv) = (step(rate_u), step(rate_v));
    let du = s.eval(u + hu, v) - s.eval(u - hu, v);
    let dv = s.eval(u, v + hv) - s.eval(u, v - hv);
    let mut n = du.cross(dv).normalized().unwrap();
    if let Some(own) = s.normal(u, v)
        && own.dot(n) < 0.0
    {
        n = -n;
    }
    if f.reversed { -n } else { n }
}

/// The ball of a plane across the edge: its centre and its feet on the
/// two faces.
#[derive(Clone, Copy, Debug)]
pub struct Ball {
    pub centre: Vec3,
    pub radius: f64,
    pub feet: [Vec3; 2],
    pub params: [(f64, f64); 2],
}

pub struct Oracle<'a> {
    pub body: &'a Body,
    pub faces: [FaceId; 2],
    pub edge: ByLength,
    pub law: Law,
    /// −1: the ball in the material (a convex edge); +1 outside it.
    pub side: f64,
}

impl Oracle<'_> {
    pub fn signed(&self, k: usize, c: Vec3, seed: (f64, f64)) -> Option<(f64, Vec3, (f64, f64))> {
        let face = self.body.face(self.faces[k]).unwrap();
        let uv = face
            .surface
            .project_from(c, seed)
            .or_else(|_| face.surface.project(c))
            .ok()?;
        let q = face.surface.eval(uv.0, uv.1);
        let n = normal(self.body, self.faces[k], uv);
        Some(((c - q).dot(n), q, uv))
    }

    /// The ball in the plane across the edge at the length `a`.
    pub fn ball(&self, a: f64) -> Option<Ball> {
        let length = self.edge.length();
        let r = self.law.r(a, length);
        let (p, t) = self.edge.at(a);
        let face = |k: usize| self.body.face(self.faces[k]).unwrap();
        let mut seeds = [
            face(0).surface.project(p).ok()?,
            face(1).surface.project(p).ok()?,
        ];
        let n0 = normal(self.body, self.faces[0], seeds[0]);
        let n1 = normal(self.body, self.faces[1], seeds[1]);
        let e1 = (n0 - t * t.dot(n0)).normalized()?;
        let e2 = t.cross(e1);
        let cos = n0.dot(n1).clamp(-1.0, 1.0);
        let half = (0.5 * (1.0 + cos)).sqrt();
        let towards = ((n0 + n1) * self.side).normalized()?;
        let towards = (towards - t * t.dot(towards)).normalized()?;
        let start = p + towards * (r / half);
        let mut x = [(start - p).dot(e1), (start - p).dot(e2)];
        let at = |x: [f64; 2]| p + e1 * x[0] + e2 * x[1];
        // Newton's method on the two signed distances, whose gradient is
        // the unit normal at the foot.
        for _ in 0..80 {
            let c = at(x);
            let (d0, _, uv0) = self.signed(0, c, seeds[0])?;
            let (d1, _, uv1) = self.signed(1, c, seeds[1])?;
            seeds = [uv0, uv1];
            let here = [d0 - self.side * r, d1 - self.side * r];
            if here[0].abs().max(here[1].abs()) <= 1e-12 * (r + 1e-4 * p.length()) {
                break;
            }
            let (m0, m1) = (
                normal(self.body, self.faces[0], uv0),
                normal(self.body, self.faces[1], uv1),
            );
            let j = [[m0.dot(e1), m0.dot(e2)], [m1.dot(e1), m1.dot(e2)]];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if !(det.abs() > 1e-12) {
                return None;
            }
            let dx = (here[0] * j[1][1] - here[1] * j[0][1]) / det;
            let dy = (j[0][0] * here[1] - j[1][0] * here[0]) / det;
            let mut step = 1.0;
            let len = dx.phypot(dy);
            if len > 0.25 * r {
                step = 0.25 * r / len;
            }
            x = [x[0] - step * dx, x[1] - step * dy];
        }
        let c = at(x);
        let (d0, q0, uv0) = self.signed(0, c, seeds[0])?;
        let (d1, q1, uv1) = self.signed(1, c, seeds[1])?;
        let off = (d0 - self.side * r).abs().max((d1 - self.side * r).abs());
        // Settled, and the ball of this edge: its centre within a few
        // radii of the edge's point.
        if !(off <= 1e-9 * r.max(1e-300) + 1e-12 * p.length()) || c.distance(p) > 6.0 * r {
            return None;
        }
        Some(Ball {
            centre: c,
            radius: r,
            feet: [q0, q1],
            params: [uv0, uv1],
        })
    }

    /// Which side of its arc's plane `p` lies on, for the ball at `a`.
    pub fn side_of(&self, p: Vec3, a: f64) -> Option<(f64, Ball)> {
        // Newton's method on the two distances is not always settled at
        // the first start: a hair along the edge, then.
        let r = self.law.radius;
        let ball = self
            .ball(a)
            .or_else(|| self.ball(a + 1e-7 * r))
            .or_else(|| self.ball(a - 1e-7 * r))
            .or_else(|| self.ball(a + 1e-5 * r))?;
        let m = (ball.feet[0] - ball.centre)
            .cross(ball.feet[1] - ball.centre)
            .normalized()?;
        Some(((p - ball.centre).dot(m), ball))
    }

    /// The ball whose arc's plane holds `p`, found by the secant rule from
    /// the length of the edge's point nearest `p`; with `p`'s distance off
    /// that ball (`|p − c| − r`) and how far outside the arc between the
    /// feet it lies, as an angle.
    pub fn place(&self, p: Vec3) -> Option<(f64, Ball, f64, f64)> {
        let a0 = self.edge.nearest(p);
        let w = 0.05 * self.law.radius.max(1e-300) + 1e-9 * self.edge.length();
        let (mut a, mut b) = (a0 - w, a0 + w);
        let (mut ga, _) = self.side_of(p, a)?;
        let (mut gb, _) = self.side_of(p, b)?;
        let mut grow = 0;
        while ga.signum() == gb.signum() && grow < 40 {
            a -= w * f64::from(1 << (grow / 2).min(20));
            b += w * f64::from(1 << (grow / 2).min(20));
            ga = self.side_of(p, a)?.0;
            gb = self.side_of(p, b)?.0;
            grow += 1;
        }
        if ga.signum() == gb.signum() {
            return None;
        }
        // Illinois' variant of the false position.
        let mut last = 0;
        let mut found = 0.5 * (a + b);
        for _ in 0..200 {
            let mut m = (a * gb - b * ga) / (gb - ga);
            // Where no ball is found at the secant's point, the middle.
            let gm = match self.side_of(p, m) {
                Some((g, _)) => g,
                None => {
                    m = 0.5 * (a + b);
                    self.side_of(p, m)?.0
                }
            };
            found = m;
            // Settled to the rounding of the side (Illinois' halving would
            // otherwise push the point back across the bracket).
            let noise = 1e-13 * (self.law.radius + 1e-3 * p.length());
            if gm.abs() <= noise || (b - a).abs() <= 1e-15 * self.edge.length().max(1e-300) {
                break;
            }
            if gm.signum() == ga.signum() {
                a = m;
                ga = gm;
                if last == -1 {
                    gb *= 0.5;
                }
                last = -1;
            } else {
                b = m;
                gb = gm;
                if last == 1 {
                    ga *= 0.5;
                }
                last = 1;
            }
        }
        let (_, ball) = self.side_of(p, found)?;
        let off = p.distance(ball.centre) - ball.radius;
        let ray = (p - ball.centre).normalized()?;
        let to = |q: Vec3| (q - ball.centre).normalized().unwrap();
        let angle = |x: Vec3, y: Vec3| x.dot(y).clamp(-1.0, 1.0).pacos();
        let span = angle(to(ball.feet[0]), to(ball.feet[1]));
        let outside = angle(ray, to(ball.feet[0])) + angle(ray, to(ball.feet[1])) - span;
        Some((found, ball, off, outside))
    }

    pub fn place_why(&self, p: Vec3) -> Option<(f64, Ball, f64, f64)> {
        let a0 = self.edge.nearest(p);
        let w = 0.05 * self.law.radius.max(1e-300) + 1e-9 * self.edge.length();
        let (mut a, mut b) = (a0 - w, a0 + w);
        let Some((mut ga, _)) = self.side_of(p, a) else {
            println!("WHY first a {a}");
            return None;
        };
        let Some((mut gb, _)) = self.side_of(p, b) else {
            println!("WHY first b {b}");
            return None;
        };
        let mut grow = 0;
        while ga.signum() == gb.signum() && grow < 40 {
            a -= w * f64::from(1 << (grow / 2).min(20));
            b += w * f64::from(1 << (grow / 2).min(20));
            ga = self.side_of(p, a)?.0;
            gb = self.side_of(p, b)?.0;
            grow += 1;
        }
        if ga.signum() == gb.signum() {
            println!("WHY no bracket {a} {b} {ga} {gb}");
            return None;
        }
        // Illinois' variant of the false position.
        let mut last = 0;
        let mut found = 0.5 * (a + b);
        for _ in 0..200 {
            let mut m = (a * gb - b * ga) / (gb - ga);
            // Where no ball is found at the secant's point, the middle.
            let gm = match self.side_of(p, m) {
                Some((g, _)) => g,
                None => {
                    m = 0.5 * (a + b);
                    let Some(x) = self.side_of(p, m) else {
                        println!("WHY middle {m} {a} {b} {ga} {gb}");
                        return None;
                    };
                    x.0
                }
            };
            found = m;
            // Settled to the rounding of the side (Illinois' halving would
            // otherwise push the point back across the bracket).
            let noise = 1e-13 * (self.law.radius + 1e-3 * p.length());
            if gm.abs() <= noise || (b - a).abs() <= 1e-15 * self.edge.length().max(1e-300) {
                break;
            }
            if gm.signum() == ga.signum() {
                a = m;
                ga = gm;
                if last == -1 {
                    gb *= 0.5;
                }
                last = -1;
            } else {
                b = m;
                gb = gm;
                if last == 1 {
                    ga *= 0.5;
                }
                last = 1;
            }
        }
        let Some((_, ball)) = self.side_of(p, found) else {
            println!("WHY found {found}");
            return None;
        };
        let off = p.distance(ball.centre) - ball.radius;
        let ray = (p - ball.centre).normalized()?;
        let to = |q: Vec3| (q - ball.centre).normalized().unwrap();
        let angle = |x: Vec3, y: Vec3| x.dot(y).clamp(-1.0, 1.0).pacos();
        let span = angle(to(ball.feet[0]), to(ball.feet[1]));
        let outside = angle(ray, to(ball.feet[0])) + angle(ray, to(ball.feet[1])) - span;
        Some((found, ball, off, outside))
    }
}

pub fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
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

pub fn fillet_faces(made: &FilletResult) -> Vec<FaceId> {
    (0..made.faces.len())
        .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
        .map(|i| FaceId(i as u32))
        .collect()
}

/// The points of a face's mesh (its own surface's points, its borders'
/// points), at the chord `chord`.
pub fn mesh_points(body: &Body, face: FaceId, chord: f64) -> Result<(Mesh, Vec<Vec3>), String> {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .map_err(|e| format!("no mesh of the face at {chord}: {e}"))?;
    let mut used = vec![false; mesh.positions.len()];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        if mesh.face_ids[t] == face.0 {
            for v in tri {
                used[*v as usize] = true;
            }
        }
    }
    let points = mesh
        .positions
        .iter()
        .zip(&used)
        .filter(|(_, u)| **u)
        .map(|(p, _)| *p)
        .collect();
    Ok((mesh, points))
}

/// The largest sine between the fillet face's normal and its neighbour's
/// along each edge it shares with a face it runs into smoothly (sine
/// under 1e-3), normals by central differences, 33 points of each; and
/// how many such borders there are.
pub fn border_sine(body: &Body, own: FaceId) -> (f64, usize) {
    let mut worst = 0.0_f64;
    let mut borders = 0;
    for index in 0..body.edges.len() {
        let edge = EdgeId(index as u32);
        let users = faces_of(body, edge);
        let [a, b] = users[..] else { continue };
        if (a == own) == (b == own) {
            continue;
        }
        let e = body.edge(edge).unwrap();
        let mut here = 0.0_f64;
        for i in 0..33 {
            let s = (f64::from(i) + 0.5) / 33.0;
            let p = e.curve.eval(e.range.0 + s * (e.range.1 - e.range.0));
            let uva = body.face(a).unwrap().surface.project(p).unwrap();
            let uvb = body.face(b).unwrap().surface.project(p).unwrap();
            let (n, m) = (normal(body, a, uva), normal(body, b, uvb));
            here = here.max(n.cross(m).length());
        }
        if here < 1e-3 {
            borders += 1;
            worst = worst.max(here);
        }
    }
    (worst, borders)
}

/// What a body must be: valid, free of crossings (the deep check's mesh
/// search), measured, and meshed watertight at `chord` with the volume
/// the measure gives. Returns the measured volume, or what failed.
pub fn sound(body: &Body, tol: &craft3d_math::Tolerance, chord: f64) -> Result<f64, String> {
    let defects = craft3d_topo::check(body, tol);
    if !defects.is_empty() {
        return Err(format!("invalid: {defects:?}"));
    }
    let crossings = craft3d_tess::body_self_intersections(
        body,
        &craft3d_tess::SelfCheckParams::new(tol.linear),
    )
    .map_err(|e| format!("self-check cannot run: {e}"))?;
    if !crossings.is_clean() {
        return Err(format!("passes through itself: {crossings}"));
    }
    let measured = craft3d_topo::mass_properties(body).map_err(|e| format!("no measure: {e:?}"))?;
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .map_err(|e| format!("no mesh at {chord}: {e}"))?;
    let welded = mesh.weld().map_err(|e| format!("no weld: {e}"))?;
    if !welded.is_closed() {
        return Err("mesh not closed".into());
    }
    let off = (welded.volume() - measured.volume).abs();
    if !(off <= measured.area * chord) {
        return Err(format!(
            "mesh volume {} against the measure {} (area {})",
            welded.volume(),
            measured.volume,
            measured.area
        ));
    }
    Ok(measured.volume)
}

pub fn frames(k: f64) -> Vec<(&'static str, Frame)> {
    let tilted = Frame::new(
        Vec3::new(137.0 * k, -211.0 * k, 89.0 * k),
        Vec3::new(0.31, -0.42, 0.85),
        Vec3::new(0.9, 0.3, -0.1),
    )
    .unwrap();
    vec![("world", Frame::WORLD), ("tilted", tilted)]
}

pub fn line(id: u64, k: f64, s: [f64; 2], e: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    }
}

pub fn cubic(id: u64, k: f64, p: [[f64; 2]; 4]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: p.iter().map(|q| [q[0] * k, q[1] * k]).collect(),
        }),
    }
}

pub fn prism(curves: &[SketchCurve], frame: &Frame, from: f64, to: f64, tol: &Tolerance) -> Body {
    let profiles = detect_profiles(curves, tol).unwrap();
    extrude(&profiles, frame, from, to, tol).unwrap().body
}

/// The polygon of `poly`, in units of the scale.
pub const POLY: [[f64; 2]; 7] = [
    [0.0, 0.0],
    [30.0, 0.0],
    [40.0, 10.0],
    [40.0, 25.0],
    [20.0, 25.0],
    [20.0, 15.0],
    [0.0, 15.0],
];

/// A prism 30 tall over `POLY`: a convex upright edge of 135° at (30, 0)
/// and a concave one of 90° at (20, 15).
pub fn poly(frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    let curves: Vec<SketchCurve> = (0..POLY.len())
        .map(|i| line(i as u64 + 1, k, POLY[i], POLY[(i + 1) % POLY.len()]))
        .collect();
    prism(&curves, frame, 0.0, 30.0 * k, tol)
}

/// An L whose inside corner at (14, 12) is between a spline wall and a
/// plane, 15 tall.
pub fn ell(frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    let curves = vec![
        line(1, k, [0.0, 0.0], [30.0, 0.0]),
        line(2, k, [30.0, 0.0], [30.0, 8.0]),
        cubic(3, k, [[30.0, 8.0], [24.0, 6.0], [19.0, 13.0], [14.0, 12.0]]),
        line(4, k, [14.0, 12.0], [14.0, 24.0]),
        line(5, k, [14.0, 24.0], [0.0, 24.0]),
        line(6, k, [0.0, 24.0], [0.0, 0.0]),
    ];
    prism(&curves, frame, 0.0, 15.0 * k, tol)
}

/// The spline of `step`'s wall: x from −5 to 45, y between about 10 and
/// 19; at the parameter ½ it is (20, 12.625).
pub const WALL: [[f64; 2]; 4] = [[-5.0, 15.0], [10.0, 25.0], [30.0, 0.0], [45.0, 11.0]];

/// A block 40 × 30 × 16 (z from −6 to 10) less the part above z = 0
/// beyond a spline wall: a concave curved edge along the wall's foot and
/// a convex one along its top, both ending slanted on the sides x = 0 and
/// x = 40.
pub fn step(frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    let block = primitives::block(
        &frame.at(frame.point(0.0, 0.0, -6.0 * k)).unwrap(),
        40.0 * k,
        30.0 * k,
        16.0 * k,
    )
    .unwrap();
    let curves = vec![
        cubic(1, k, WALL),
        line(2, k, [45.0, 11.0], [45.0, 40.0]),
        line(3, k, [45.0, 40.0], [-5.0, 40.0]),
        line(4, k, [-5.0, 40.0], [-5.0, 15.0]),
    ];
    let tool = prism(&curves, frame, 0.0, 20.0 * k, tol);
    boolean(&block, &tool, BooleanOp::Subtract, tol)
        .unwrap()
        .body
}

/// A frustum (cone of radius 10 and height 30, from z = 0 to 15) cut by
/// the plane y = 2.5: two curved edges where the plane meets the cone.
pub fn cut_cone(frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    let cone = primitives::cone(frame, 10.0 * k, 30.0 * k).unwrap();
    let block = primitives::block(
        &frame.at(frame.point(-20.0 * k, 2.5 * k, 0.0)).unwrap(),
        40.0 * k,
        30.0 * k,
        15.0 * k,
    )
    .unwrap();
    boolean(&cone, &block, BooleanOp::Intersect, tol)
        .unwrap()
        .body
}

/// The edge that passes nearest `p`.
pub fn edge_through(body: &Body, p: Vec3) -> (EdgeId, f64) {
    let mut best = (EdgeId(0), f64::INFINITY);
    for (i, e) in body.edges.iter().enumerate() {
        for j in 0..=256 {
            let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 256.0;
            let d = e.curve.eval(t).distance(p);
            if d < best.1 {
                best = (EdgeId(i as u32), d);
            }
        }
    }
    best
}

pub fn radius_law(law: &Law, k: f64) -> RadiusLaw {
    RadiusLaw {
        radius: law.radius * k,
        fades: [0, 1].map(|i| {
            law.fades[i].map(|(to, over)| Fade {
                to: to * k,
                over: over * k,
                back: law.backs[i] * k,
            })
        }),
    }
}

/// What the oracle found of one fillet.
#[derive(Debug, Default)]
pub struct Found {
    pub points: usize,
    pub past: usize,
    pub unplaced: usize,
    /// In tolerances: the largest distance off the ball of the plane that
    /// holds the point, and outside the arc between its feet; within the
    /// edge, and past its ends.
    pub off: f64,
    pub outside: f64,
    pub off_past: f64,
    pub outside_past: f64,
    pub sine: f64,
    pub borders: usize,
    pub volume: f64,
}

/// `law` (in units of `k`) on `edge` of `body`; the body held to being
/// sound and the fillet face to the oracle. `convex`: the ball in the
/// material. `corner`: for an edge between two planes, the angle of the
/// corner the fillet rounds (the material's for a convex edge, the empty
/// space's for a concave one), whose volume is then held to the closed
/// form of the section integrated by Simpson's rule.
pub fn held(
    what: &str,
    body: &Body,
    edge: EdgeId,
    law: Law,
    k: f64,
    convex: bool,
    corner: Option<f64>,
    chord: f64,
) -> Result<Found, String> {
    let tol = tolerance(k);
    let before = craft3d_topo::mass_properties(body).unwrap().volume;
    let made = fillet_by_law(
        body,
        &[FilletLawEdge {
            edge,
            law: radius_law(&law, k),
        }],
        &tol,
    )
    .map_err(|e| format!("{what}: refused: {e}"))?;
    // A chord of zero: the body is not held to being sound here (the
    // caller looks at that itself), only the face to the oracle.
    let volume = if chord > 0.0 {
        sound(&made.body, &tol, chord).map_err(|e| format!("{what}: {e}"))?
    } else {
        craft3d_topo::mass_properties(&made.body).map_or(f64::NAN, |m| m.volume)
    };
    let faces = faces_of(body, edge);
    let [f0, f1] = faces[..] else {
        return Err(format!("{what}: {} faces", faces.len()));
    };
    // The faces beside the fillet keep their numbers in the result where
    // they are kept: find them by their surfaces.
    let same = |old: craft3d_topo::FaceId| {
        (0..made.faces.len())
            .find(|i| matches!(&made.faces[*i], FilletFace::Kept(list) if list.contains(&old)))
            .map(|i| craft3d_topo::FaceId(i as u32))
            .unwrap()
    };
    let oracle = Oracle {
        body: &made.body,
        faces: [same(f0), same(f1)],
        edge: ByLength::new(body, edge),
        law: Law {
            radius: law.radius * k,
            fades: law.fades.map(|f| f.map(|(to, over)| (to * k, over * k))),
            backs: law.backs.map(|b| b * k),
        },
        side: if convex { -1.0 } else { 1.0 },
    };
    let length = oracle.edge.length();
    let own = fillet_faces(&made);
    let [own] = own[..] else {
        return Err(format!("{what}: {} fillet faces", own.len()));
    };
    let (_, points) = mesh_points(
        &made.body,
        own,
        if chord > 0.0 { 2.5 * chord } else { 0.1 * k },
    )
    .map_err(|e| format!("{what}: {e}"))?;
    let mut found = Found::default();
    for p in points {
        found.points += 1;
        let Some((a, ball, off, outside)) = oracle.place(p) else {
            found.unplaced += 1;
            let a0 = oracle.edge.nearest(p);
            let r0 = oracle.law.radius;
            let samples: Vec<String> = (-8..=8)
                .map(|i| {
                    let a = a0 + f64::from(i) * 0.25 * r0;
                    format!(
                        "{:.3}:{:?}",
                        a / r0,
                        oracle.side_of(p, a).map(|(g, _)| g / r0)
                    )
                })
                .collect();
            println!("UNPLACED {what}: sides {}", samples.join(" "));
            let _ = oracle.place_why(p);
            println!(
                "UNPLACED {what}: {p:?} nearest length {a0} of {length}, ball there {:?}, distance to the edge's point {}",
                oracle.ball(a0).map(|b| b.radius),
                oracle.edge.at(a0).0.distance(p)
            );
            continue;
        };
        let (off, outside) = (off.abs() / tol.linear, outside * ball.radius / tol.linear);
        if off > 1.0 || outside > 1.0 {
            let (q, _) = oracle.edge.at(a.clamp(0.0, length));
            println!(
                "FAR {what}: {p:?} at length {a} of {length} (radius {}), off {off} outside {outside} tolerances; {} from the edge's point, feet {:?}",
                ball.radius,
                q.distance(p),
                ball.feet.map(|f| f.distance(p))
            );
            for k in 0..2 {
                let fresh = oracle.signed(k, ball.centre, ball.params[k]);
                let surface = &made.body.face(oracle.faces[k]).unwrap().surface;
                println!(
                    "FAR   face {k} {:?}: |c - foot| {} signed {:?} global project {:?} domain {:?} params {:?}",
                    std::mem::discriminant(surface),
                    ball.centre.distance(ball.feet[k]),
                    fresh.map(|f| f.0),
                    surface
                        .project(ball.centre)
                        .map(|uv| surface.eval(uv.0, uv.1).distance(ball.centre)),
                    surface.domain(),
                    ball.params[k]
                );
            }
        }
        let past = a < -1e-9 * length || a > length * (1.0 + 1e-9);
        if past {
            found.past += 1;
            found.off_past = found.off_past.max(off);
            found.outside_past = found.outside_past.max(outside);
        } else {
            found.off = found.off.max(off);
            found.outside = found.outside.max(outside);
        }
    }
    let (sine, borders) = border_sine(&made.body, own);
    found.sine = sine;
    found.borders = borders;
    if let Some(theta) = corner {
        let half = 0.5 * theta;
        let section = half.pcos() / half.psin() - 0.5 * (PI - theta);
        let n = 20_000;
        let h = length / f64::from(n);
        let mut integral = 0.0;
        for i in 0..=n {
            let w = if i == 0 || i == n {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            let r = oracle.law.r(f64::from(i) * h, length);
            integral += w * r * r;
        }
        integral *= h / 3.0;
        let sign = if convex { -1.0 } else { 1.0 };
        let expected = before + sign * section * integral;
        found.volume = match craft3d_topo::mass_properties(&made.body) {
            Ok(m) => (volume - expected).abs() / (m.area * tol.linear),
            Err(_) => f64::NAN,
        };
    }
    Ok(found)
}
