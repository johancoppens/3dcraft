//! Helpers for the assembly tests: planar linkages built in 3D from pin
//! positions, closed-form circle intersections, and the three scales.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use craft3d_assembly::{Frame, Mat3, MateKind, MateSpec, Pose, Problem};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// The scales every closed-form test runs at (AGENTS.md rule 3).
pub const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A problem whose instance 0 is grounded: lengths are given at scale 1
/// and multiplied by `s`. Each body's own coordinates are the world's at
/// the reference configuration, so a pin at world point `p` is the frame
/// `Frame::at(p)` on both bodies.
pub fn planar(bodies: usize, s: f64, size: f64) -> Problem {
    let mut p = Problem::new(bodies, 1e-9 * size * s, size * s);
    p.grounded = vec![0];
    p
}

pub fn v2(x: f64, y: f64) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

/// A revolute about z between `a` and `b` at the world point `at`.
pub fn pin(p: &mut Problem, a: usize, b: usize, at: Vec3, name: &str) -> usize {
    p.mates.push(
        MateSpec::new(MateKind::Revolute, (a, Frame::at(at)), (b, Frame::at(at))).named(name),
    );
    p.mates.len() - 1
}

/// Moves every start pose but the grounded one's by a small, fixed,
/// irregular amount (`fraction` of the scale), so the solve has work to do.
pub fn disturb(p: &mut Problem, fraction: f64) {
    let l = p.scale;
    for (i, pose) in p.start.iter_mut().enumerate() {
        if p.grounded.contains(&i) {
            continue;
        }
        let k = i as f64;
        let t =
            Vec3::new((k * 1.3).psin(), (k * 2.1).pcos(), (k * 0.7).psin() * 0.5) * (fraction * l);
        let w = Vec3::new(
            (k * 0.9).pcos() * 0.1,
            (k * 1.7).psin() * 0.1,
            (k * 0.3).psin(),
        ) * fraction;
        *pose = Pose::new(Mat3::exp(w).mul(&pose.rotation), pose.translation + t);
    }
}

/// The intersection of circle (P, r) and (Q, s) on branch `sg` (+1 left of
/// P→Q), as the `strandbeest-leg` Jig's generator computes it.
pub fn circ(p: Vec3, r: f64, q: Vec3, s: f64, sg: f64) -> Option<Vec3> {
    let (dx, dy) = (q.x - p.x, q.y - p.y);
    let d = dx.phypot(dy);
    let al = (r * r - s * s + d * d) / (2.0 * d);
    let disc = 1.0 - (al / r).ppowi(2);
    if disc < 0.0 {
        return None;
    }
    let h = r * disc.sqrt();
    Some(v2(
        p.x + (al * dx - sg * h * dy) / d,
        p.y + (al * dy + sg * h * dx) / d,
    ))
}

/// The turn of a pose about z (its x axis's angle in the xy plane).
pub fn turn(pose: &Pose) -> f64 {
    let x = pose.rotation.x();
    x.y.patan2(x.x)
}

pub fn close(a: Vec3, b: Vec3, tol: f64) -> bool {
    a.distance(b) <= tol
}

/// Theo Jansen's numbers (his units; the crank m = 15).
pub const HOLY: Holy = Holy {
    a: 38.0,
    b: 41.5,
    c: 39.3,
    d: 40.1,
    e: 55.8,
    f: 39.4,
    g: 36.7,
    h: 65.7,
    i: 49.0,
    j: 50.0,
    k: 61.9,
    l: 7.8,
    m: 15.0,
};

#[derive(Clone, Copy, Debug)]
pub struct Holy {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
    pub g: f64,
    pub h: f64,
    pub i: f64,
    pub j: f64,
    pub k: f64,
    pub l: f64,
    pub m: f64,
}

/// The joints O, A, B, C, D, E, F, G of Jansen's leg at crank angle `th`,
/// scaled by `s`, by circle intersections on the `strandbeest-leg` Jig's
/// branches (C −1, D +1, E +1, F −1, G −1), in the plane z = `z`.
pub fn jansen_joints(th: f64, s: f64, z: f64) -> Option<[Vec3; 8]> {
    let n = HOLY;
    let o = v2(0.0, 0.0);
    let b = v2(-n.a * s, -n.l * s);
    let a = v2(n.m * s * th.pcos(), n.m * s * th.psin());
    let c = circ(a, n.j * s, b, n.b * s, -1.0)?;
    let d = circ(b, n.d * s, c, n.e * s, 1.0)?;
    let e = circ(a, n.k * s, b, n.c * s, 1.0)?;
    let f = circ(d, n.f * s, e, n.g * s, -1.0)?;
    let g = circ(f, n.h * s, e, n.i * s, -1.0)?;
    let lift = Vec3::new(0.0, 0.0, z);
    Some([o, a, b, c, d, e, f, g].map(|p| p + lift))
}

/// Adds one leg on `crank` (its pin A on the crank at the leg's reference
/// angle `th`), bodies `first..first + 6` (bar j, bar k, the hip triangle
/// BCD, bar c, bar f, the foot triangle EFG), in the plane z = `z`.
/// Returns the foot's instance and the foot point G in its coordinates.
pub fn add_leg(
    p: &mut Problem,
    crank: usize,
    first: usize,
    th: f64,
    s: f64,
    z: f64,
) -> (usize, Vec3) {
    let [_, a, b, c, d, e, f, g] = jansen_joints(th, s, z).unwrap();
    let (bj, bk, hip, bc, bf, foot) =
        (first, first + 1, first + 2, first + 3, first + 4, first + 5);
    pin(p, crank, bj, a, "A j");
    pin(p, crank, bk, a, "A k");
    pin(p, bj, hip, c, "C");
    pin(p, 0, hip, b, "B hip");
    pin(p, 0, bc, b, "B c");
    pin(p, bk, bc, e, "E k");
    pin(p, bc, foot, e, "E foot");
    pin(p, hip, bf, d, "D");
    pin(p, bf, foot, f, "F");
    (foot, g)
}
