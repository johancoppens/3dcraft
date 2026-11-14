//! Shared by the seeded hunts of the design checks (`hunt_checks_*.rs`):
//! a seeded generator, documents in mm at a tolerance of a millionth of
//! the scale, commands run with their panics caught, rigid motions that
//! take a case to another frame (turned about a random axis and moved far
//! from the origin), and closed-form oracles of the test's own (vectors,
//! the distance of a point from a box, a cylinder and a segment, of two
//! segments, of a point from a rectangle, the separating axes of two
//! boxes, and points of a cubic Bézier curve), none of which uses the
//! code under test.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// SplitMix64, as the kernel's hunts use it (rule 5: seeded only).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// One of `items`.
    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }

    /// A unit vector, uniform on the sphere.
    pub fn direction(&mut self) -> V {
        loop {
            let v = V::new(
                self.between(-1.0, 1.0),
                self.between(-1.0, 1.0),
                self.between(-1.0, 1.0),
            );
            let l = v.len();
            if l > 0.1 && l <= 1.0 {
                return v * (1.0 / l);
            }
        }
    }
}

/// A vector of the test's own, so that no oracle leans on the vectors of
/// the code under test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        V { x, y, z }
    }
    pub fn dot(self, o: V) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V) -> V {
        V::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn unit(self) -> V {
        self * (1.0 / self.len())
    }
    pub fn max_abs(self) -> f64 {
        self.x.abs().max(self.y.abs()).max(self.z.abs())
    }
    pub fn get(self, i: usize) -> f64 {
        [self.x, self.y, self.z][i]
    }
    pub fn axis(i: usize) -> V {
        [
            V::new(1.0, 0.0, 0.0),
            V::new(0.0, 1.0, 0.0),
            V::new(0.0, 0.0, 1.0),
        ][i]
    }
    /// Some unit vector square to this one.
    pub fn square(self) -> V {
        let a = if self.x.abs() < 0.6 {
            V::new(1.0, 0.0, 0.0)
        } else {
            V::new(0.0, 1.0, 0.0)
        };
        self.cross(a).unit()
    }
}

impl std::ops::Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        V::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        V::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl std::ops::Mul<f64> for V {
    type Output = V;
    fn mul(self, s: f64) -> V {
        V::new(self.x * s, self.y * s, self.z * s)
    }
}
impl std::ops::Neg for V {
    type Output = V;
    fn neg(self) -> V {
        V::new(-self.x, -self.y, -self.z)
    }
}

/// A turn by `deg` degrees about `axis` (right-handed, Rodrigues).
#[derive(Clone, Copy, Debug)]
pub struct Turn {
    pub axis: V,
    pub deg: f64,
}

impl Turn {
    pub const NONE: Turn = Turn {
        axis: V::new(0.0, 0.0, 1.0),
        deg: 0.0,
    };
    pub fn apply(&self, v: V) -> V {
        let k = self.axis.unit();
        let t = self.deg.to_radians();
        let (s, c) = (t.psin(), t.pcos());
        v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c))
    }
    pub fn random(rng: &mut Rng) -> Turn {
        Turn {
            axis: rng.direction(),
            deg: rng.between(-180.0, 180.0),
        }
    }
}

/// A length in mm, to the last bit.
pub fn mm(x: f64) -> String {
    format!("{x:?} mm")
}

pub fn mm3(v: V) -> Value {
    json!([mm(v.x), mm(v.y), mm(v.z)])
}

pub fn num3(v: V) -> Value {
    json!([v.x, v.y, v.z])
}

/// A command and its parameters.
pub type Step = (&'static str, Value);

/// A document in mm at a tolerance of a millionth of the scale `k`.
pub fn document(k: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    s.execute("file.new", &json!({"units": "mm", "tolerance": 1e-6 * k}))
        .unwrap();
    s
}

/// What a command gave: its answer, its error, or the panic it raised.
#[derive(Clone, Debug)]
pub enum Ran {
    Ok(Value),
    Err(String),
    Panic(String),
}

pub fn run(s: &mut Session, id: &str, params: &Value) -> Ran {
    match catch_unwind(AssertUnwindSafe(|| s.execute(id, params))) {
        Ok(Ok(v)) => Ran::Ok(v),
        Ok(Err(e)) => Ran::Err(e.to_string()),
        Err(p) => Ran::Panic(
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default(),
        ),
    }
}

/// Runs the steps; the first that fails or panics is returned.
pub fn build(s: &mut Session, steps: &[Step]) -> Result<(), Ran> {
    for (id, params) in steps {
        match run(s, id, params) {
            Ran::Ok(_) => {}
            other => return Err(other),
        }
    }
    Ok(())
}

/// A rigid motion of a whole case: a turn about the origin, then a move.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub turn: Turn,
    pub shift: V,
}

impl Motion {
    pub const NONE: Motion = Motion {
        turn: Turn::NONE,
        shift: V::new(0.0, 0.0, 0.0),
    };

    /// The world (one in four), else turned about a random axis and moved
    /// up to `far` times the scale away.
    pub fn random(rng: &mut Rng, k: f64) -> Motion {
        if rng.below(4) == 0 {
            return Motion::NONE;
        }
        let far = rng.pick(&[0.0, 30.0, 1000.0, 20000.0]);
        Motion {
            turn: Turn::random(rng),
            shift: rng.direction() * (far * k),
        }
    }

    pub fn apply(&self, v: V) -> V {
        self.turn.apply(v) + self.shift
    }

    /// The `feature.transform` that moves `source` so, naming the result.
    pub fn step(&self, source: &str, name: &str) -> Step {
        (
            "feature.transform",
            json!({
                "source": source,
                "name": name,
                "angle": format!("{:?} deg", self.turn.deg),
                "axis": num3(self.turn.axis),
                "translate": mm3(self.shift),
            }),
        )
    }

    /// How far from the origin the case lies, for the rounding a far
    /// frame brings.
    pub fn reach(&self) -> f64 {
        self.shift.len()
    }

    pub fn describe(&self) -> String {
        if self.turn.deg == 0.0 && self.shift.len() == 0.0 {
            "world".into()
        } else {
            format!(
                "turned {:.3} deg about [{:.4}, {:.4}, {:.4}], moved {:.4e}",
                self.turn.deg,
                self.turn.axis.x,
                self.turn.axis.y,
                self.turn.axis.z,
                self.shift.len()
            )
        }
    }
}

/// The nearest point of the box `[lo, hi]` to `p` (itself when inside).
pub fn box_nearest(lo: V, hi: V, p: V) -> V {
    V::new(
        p.x.clamp(lo.x, hi.x),
        p.y.clamp(lo.y, hi.y),
        p.z.clamp(lo.z, hi.z),
    )
}

/// How far `p` lies from the box `[lo, hi]` (0 inside).
pub fn box_distance(lo: V, hi: V, p: V) -> f64 {
    (p - box_nearest(lo, hi, p)).len()
}

/// How far a point inside the box lies from its boundary.
pub fn box_depth(lo: V, hi: V, p: V) -> f64 {
    (p.x - lo.x)
        .min(hi.x - p.x)
        .min(p.y - lo.y)
        .min(hi.y - p.y)
        .min(p.z - lo.z)
        .min(hi.z - p.z)
}

/// How far `p` lies from the solid cylinder of radius `r` along z from 0
/// to `h` (0 inside).
pub fn cylinder_distance(r: f64, h: f64, p: V) -> f64 {
    let rho = p.x.phypot(p.y);
    let dr = (rho - r).max(0.0);
    let dz = (p.z - h).max(-p.z).max(0.0);
    dr.phypot(dz)
}

/// The nearest point of the segment `a b` to `p`.
pub fn segment_nearest(a: V, b: V, p: V) -> V {
    let d = b - a;
    let l2 = d.dot(d);
    if l2 == 0.0 {
        return a;
    }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    a + d * t
}

/// The least distance of two segments (after Ericson's closest points of
/// two segments, clamped; the degenerate cases by the point and segment).
pub fn segment_segment(p1: V, q1: V, p2: V, q2: V) -> f64 {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 0.0 && e <= 0.0 {
        return (p1 - p2).len();
    }
    if a <= 0.0 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 0.0 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut ss = if denom > 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 {
                tt = 0.0;
                ss = (-c / a).clamp(0.0, 1.0);
            } else if tt > 1.0 {
                tt = 1.0;
                ss = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = ss;
            t = tt;
        }
    }
    let c1 = p1 + d1 * s;
    let c2 = p2 + d2 * t;
    // Both checked against the other segment once more, so a clamp that
    // left one end off its best is mended.
    let a1 = (c1 - segment_nearest(p2, q2, c1)).len();
    let a2 = (c2 - segment_nearest(p1, q1, c2)).len();
    (c1 - c2).len().min(a1).min(a2)
}

/// A box placed in space: its centre, its three unit axes and half sizes.
#[derive(Clone, Copy, Debug)]
pub struct Obb {
    pub c: V,
    pub u: [V; 3],
    pub h: [f64; 3],
}

impl Obb {
    pub fn axis_aligned(lo: V, hi: V) -> Obb {
        Obb {
            c: (lo + hi) * 0.5,
            u: [V::axis(0), V::axis(1), V::axis(2)],
            h: [
                0.5 * (hi.x - lo.x),
                0.5 * (hi.y - lo.y),
                0.5 * (hi.z - lo.z),
            ],
        }
    }

    pub fn corner(&self, i: usize) -> V {
        let s = |b: usize| if i & (1 << b) != 0 { 1.0 } else { -1.0 };
        self.c
            + self.u[0] * (s(0) * self.h[0])
            + self.u[1] * (s(1) * self.h[1])
            + self.u[2] * (s(2) * self.h[2])
    }

    pub fn corners(&self) -> [V; 8] {
        std::array::from_fn(|i| self.corner(i))
    }

    /// The twelve edges, as pairs of corners.
    pub fn edges(&self) -> Vec<(V, V)> {
        let mut out = Vec::new();
        for i in 0..8 {
            for b in 0..3 {
                if i & (1 << b) == 0 {
                    out.push((self.corner(i), self.corner(i | (1 << b))));
                }
            }
        }
        out
    }

    /// Local coordinates of `p`.
    pub fn local(&self, p: V) -> V {
        let d = p - self.c;
        V::new(d.dot(self.u[0]), d.dot(self.u[1]), d.dot(self.u[2]))
    }

    /// How far `p` lies from this solid box (0 inside).
    pub fn distance(&self, p: V) -> f64 {
        let l = self.local(p);
        let h = V::new(self.h[0], self.h[1], self.h[2]);
        box_distance(-h, h, l)
    }

    /// How deep `p` lies inside (negative outside along the nearest
    /// axis; only meaningful inside).
    pub fn depth(&self, p: V) -> f64 {
        let l = self.local(p);
        let h = V::new(self.h[0], self.h[1], self.h[2]);
        box_depth(-h, h, l)
    }

    pub fn contains(&self, p: V) -> bool {
        self.depth(p) > 0.0
    }

    /// The six faces, each as a corner and its two edge vectors.
    pub fn faces(&self) -> Vec<(V, V, V)> {
        let mut out = Vec::new();
        for i in 0..3 {
            let (j, k) = ((i + 1) % 3, (i + 2) % 3);
            for s in [-1.0, 1.0] {
                let o = self.c + self.u[i] * (s * self.h[i])
                    - self.u[j] * self.h[j]
                    - self.u[k] * self.h[k];
                out.push((
                    o,
                    self.u[j] * (2.0 * self.h[j]),
                    self.u[k] * (2.0 * self.h[k]),
                ));
            }
        }
        out
    }
}

/// How far `p` lies from the rectangle at `o` spanned by the square
/// vectors `e1` and `e2`.
pub fn rect_distance(o: V, e1: V, e2: V, p: V) -> f64 {
    let d = p - o;
    let s = (d.dot(e1) / e1.dot(e1)).clamp(0.0, 1.0);
    let t = (d.dot(e2) / e2.dot(e2)).clamp(0.0, 1.0);
    (p - (o + e1 * s + e2 * t)).len()
}

/// Whether two boxes share volume (strictly): no separating axis among
/// the fifteen of two boxes, by more than `slack`.
pub fn obb_overlap(a: &Obb, b: &Obb, slack: f64) -> bool {
    let mut axes: Vec<V> = Vec::new();
    axes.extend(a.u);
    axes.extend(b.u);
    for i in 0..3 {
        for j in 0..3 {
            let c = a.u[i].cross(b.u[j]);
            if c.len() > 1e-9 {
                axes.push(c.unit());
            }
        }
    }
    let t = b.c - a.c;
    for ax in axes {
        let ra: f64 = (0..3).map(|i| a.h[i] * a.u[i].dot(ax).abs()).sum();
        let rb: f64 = (0..3).map(|i| b.h[i] * b.u[i].dot(ax).abs()).sum();
        if t.dot(ax).abs() >= ra + rb - slack {
            return false;
        }
    }
    true
}

/// The least distance of two boxes that do not share volume: the least,
/// over the corners of each against the faces of the other and the edges
/// of each against the edges of the other (two convex polyhedra apart are
/// nearest at such a pair of features).
pub fn obb_distance(a: &Obb, b: &Obb) -> f64 {
    let mut best = f64::INFINITY;
    for (x, y) in [(a, b), (b, a)] {
        for p in x.corners() {
            for (o, e1, e2) in y.faces() {
                best = best.min(rect_distance(o, e1, e2, p));
            }
        }
    }
    for (p1, q1) in a.edges() {
        for (p2, q2) in b.edges() {
            best = best.min(segment_segment(p1, q1, p2, q2));
        }
    }
    best
}

/// The least distance of the boundaries of two boxes that share volume:
/// 0 where the boundaries cross, else (one wholly inside the other) the
/// least depth of the inner one's corners in the outer.
pub fn obb_boundary_distance(a: &Obb, b: &Obb) -> f64 {
    for (inner, outer) in [(a, b), (b, a)] {
        if inner.corners().iter().all(|&p| outer.contains(p)) {
            return inner
                .corners()
                .iter()
                .map(|&p| outer.depth(p))
                .fold(f64::INFINITY, f64::min);
        }
    }
    0.0
}

/// A cubic Bézier curve in a plane.
#[derive(Clone, Copy, Debug)]
pub struct Bezier(pub [[f64; 2]; 4]);

impl Bezier {
    pub fn at(&self, t: f64) -> [f64; 2] {
        let s = 1.0 - t;
        let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        let p = &self.0;
        [
            w[0] * p[0][0] + w[1] * p[1][0] + w[2] * p[2][0] + w[3] * p[3][0],
            w[0] * p[0][1] + w[1] * p[1][1] + w[2] * p[2][1] + w[3] * p[3][1],
        ]
    }

    pub fn d(&self, t: f64) -> [f64; 2] {
        let s = 1.0 - t;
        let p = &self.0;
        let w = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
        [
            w[0] * (p[1][0] - p[0][0]) + w[1] * (p[2][0] - p[1][0]) + w[2] * (p[3][0] - p[2][0]),
            w[0] * (p[1][1] - p[0][1]) + w[1] * (p[2][1] - p[1][1]) + w[2] * (p[3][1] - p[2][1]),
        ]
    }

    /// The least distance from `q` to the curve, and the parameter: dense
    /// samples, then the best few refined by golden sections on their
    /// brackets (the distance is smooth; the refined value is a real point
    /// of the curve, so never below the truth, and the dense samples are
    /// `step` apart in t, so no minimum narrower than that is missed).
    pub fn nearest(&self, q: [f64; 2]) -> (f64, f64) {
        let dist = |t: f64| {
            let p = self.at(t);
            (p[0] - q[0]).phypot(p[1] - q[1])
        };
        let n = 4000;
        let samples: Vec<f64> = (0..=n).map(|i| dist(i as f64 / n as f64)).collect();
        let mut best = (f64::INFINITY, 0.0);
        for i in 0..=n {
            let left = i == 0 || samples[i] <= samples[i - 1];
            let right = i == n || samples[i] <= samples[i + 1];
            if left && right {
                let lo = (i as f64 - 1.0).max(0.0) / n as f64;
                let hi = (i as f64 + 1.0).min(n as f64) / n as f64;
                let (t, d) = golden(lo, hi, dist);
                if d < best.0 {
                    best = (d, t);
                }
            }
        }
        best
    }
}

/// The least of `f` on `[lo, hi]` by golden sections (f unimodal there),
/// the ends included.
pub fn golden(mut lo: f64, mut hi: f64, f: impl Fn(f64) -> f64) -> (f64, f64) {
    let (flo, fhi) = (f(lo), f(hi));
    let g = 0.5 * (5.0_f64.sqrt() - 1.0);
    let mut a = hi - g * (hi - lo);
    let mut b = lo + g * (hi - lo);
    let (mut fa, mut fb) = (f(a), f(b));
    for _ in 0..200 {
        if fa < fb {
            hi = b;
            b = a;
            fb = fa;
            a = hi - g * (hi - lo);
            fa = f(a);
        } else {
            lo = a;
            a = b;
            fa = fb;
            b = lo + g * (hi - lo);
            fb = f(b);
        }
        if hi - lo < 1e-17 {
            break;
        }
    }
    let mut best = if fa < fb { (a, fa) } else { (b, fb) };
    if flo < best.1 {
        best = (lo.min(a), flo);
    }
    if fhi < best.1 {
        best = (hi.max(b), fhi);
    }
    best
}

/// The least distance of two cubic Bézier curves: for points of `a` a
/// thousandth of its parameter apart, the nearest point of `b`
/// ([`Bezier::nearest`]); then the local least of those refined by golden
/// sections on `a`'s parameter, each step again the nearest point of `b`.
/// Every value is a real pair of points, so never below the truth; the
/// distance from `b` is a continuous function of `a`'s parameter, so no
/// least wider than the spacing is missed. (Alternating sections on each
/// curve in turn stall where the curves run nearly parallel: a crossing
/// of 2.5e-8 was read as 5.4e-4 apart.)
pub fn bezier_bezier(a: &Bezier, b: &Bezier) -> f64 {
    let f = |s: f64| b.nearest(a.at(s)).0;
    let n = 1000;
    let samples: Vec<f64> = (0..=n).map(|i| f(i as f64 / n as f64)).collect();
    let mut best = f64::INFINITY;
    for i in 0..=n {
        let left = i == 0 || samples[i] <= samples[i - 1];
        let right = i == n || samples[i] <= samples[i + 1];
        if left && right {
            let lo = (i as f64 - 1.0).max(0.0) / n as f64;
            let hi = (i as f64 + 1.0).min(n as f64) / n as f64;
            best = best.min(golden(lo, hi, f).1);
        }
    }
    best
}

/// The least of `f` over `[lo, hi]`: dense samples, the local least
/// refined by golden sections (for a smooth `f`).
pub fn least_of(lo: f64, hi: f64, f: impl Fn(f64) -> f64) -> (f64, f64) {
    let n = 20000;
    let x = |i: usize| lo + (hi - lo) * i as f64 / n as f64;
    let samples: Vec<f64> = (0..=n).map(|i| f(x(i))).collect();
    let mut best = (f64::INFINITY, lo);
    for i in 0..=n {
        let left = i == 0 || samples[i] <= samples[i - 1];
        let right = i == n || samples[i] <= samples[i + 1];
        if left && right {
            let (t, v) = golden(x(i.saturating_sub(1)), x((i + 1).min(n)), &f);
            if v < best.0 {
                best = (v, t);
            }
        }
    }
    best
}

/// The least distance from the curve `b` to the segment `p q` in its
/// plane: dense samples, the best refined by golden sections (a real point
/// of the curve: never below the truth).
pub fn bezier_segment(b: &Bezier, p: [f64; 2], q: [f64; 2]) -> f64 {
    let (p3, q3) = (V::new(p[0], p[1], 0.0), V::new(q[0], q[1], 0.0));
    let dist = |t: f64| {
        let c = b.at(t);
        let c = V::new(c[0], c[1], 0.0);
        (c - segment_nearest(p3, q3, c)).len()
    };
    let n = 4000;
    let samples: Vec<f64> = (0..=n).map(|i| dist(i as f64 / n as f64)).collect();
    let mut best = f64::INFINITY;
    for i in 0..=n {
        let left = i == 0 || samples[i] <= samples[i - 1];
        let right = i == n || samples[i] <= samples[i + 1];
        if left && right {
            let lo = (i as f64 - 1.0).max(0.0) / n as f64;
            let hi = (i as f64 + 1.0).min(n as f64) / n as f64;
            best = best.min(golden(lo, hi, dist).1);
        }
    }
    best
}

/// What one case came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every answer held against the oracle.
    Right,
    /// The check declined (an error with a reason, or not proven).
    Declined,
    /// An answer is false.
    Wrong,
    /// The check panicked.
    Panic,
    /// The answer moved with the frame by more than it may.
    Frame,
}

impl Verdict {
    pub fn letter(self) -> char {
        match self {
            Verdict::Right => 'R',
            Verdict::Declined => 'D',
            Verdict::Wrong => 'W',
            Verdict::Panic => 'P',
            Verdict::Frame => 'F',
        }
    }
}

/// The counts of a sweep, and the lines of what was not right.
#[derive(Default)]
pub struct Tally {
    pub cases: usize,
    pub right: usize,
    pub declined: usize,
    pub wrong: usize,
    pub panic: usize,
    pub frame: usize,
    /// Right, but a value found a few tolerances off where the command
    /// says exact.
    pub inexact: usize,
    /// (case id, letter, what) for every case not right.
    pub notes: Vec<(String, char, String)>,
    pub letters: String,
}

impl Tally {
    pub fn add(&mut self, id: &str, verdict: Verdict, what: String) {
        self.cases += 1;
        self.letters.push(verdict.letter());
        match verdict {
            Verdict::Right => self.right += 1,
            Verdict::Declined => self.declined += 1,
            Verdict::Wrong => self.wrong += 1,
            Verdict::Panic => self.panic += 1,
            Verdict::Frame => self.frame += 1,
        }
        if verdict != Verdict::Right {
            self.notes.push((id.to_string(), verdict.letter(), what));
        }
    }

    pub fn merge(&mut self, o: Tally) {
        self.cases += o.cases;
        self.right += o.right;
        self.declined += o.declined;
        self.wrong += o.wrong;
        self.panic += o.panic;
        self.frame += o.frame;
        self.inexact += o.inexact;
        self.notes.extend(o.notes);
        self.letters.push_str(&o.letters);
    }

    pub fn summary(&self, name: &str) -> String {
        format!(
            "{name}: {} cases, {} right ({} inexact by a few tolerances), {} declined, {} WRONG, {} panic, {} frame-dependent",
            self.cases, self.right, self.inexact, self.declined, self.wrong, self.panic, self.frame
        )
    }

    pub fn print(&self, name: &str) {
        println!("{}", self.summary(name));
        println!("  letters: {}", self.letters);
        for (id, letter, what) in &self.notes {
            println!("  {letter} {id}: {what}");
        }
    }
}
