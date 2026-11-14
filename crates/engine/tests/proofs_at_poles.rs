//! A seeded hunt of what `measure.clearance`, `measure.distance` and
//! `measure.thickness` prove next to the places where a face has no edge
//! although something turns there: a cone's apex, a sphere's poles, a
//! torus (no edge at all), the tip of an arc or a spline turned about an
//! axis it ends on, each also as a pit (concave) and as a part of a turn
//! (where the tip is a vertex), beside the same shapes without such a
//! place (a frustum).
//!
//! Every body is a profile of lines, arcs and a cubic turned about an
//! axis, so the test knows its surface in closed form: a point of the
//! profile in the half-plane `(ρ, z)` turned by any angle is a point of
//! the body. Against it:
//! - **a ball**, whose distance from the body is its centre's distance
//!   from the profile in that half-plane less its radius (closed form
//!   for lines and arcs, golden sections on dense samples for the cubic):
//!   placed off a side, a rim, an apex or a pole at gaps from three sizes
//!   down to a billionth, touching, and sunk in; of a radius from a third
//!   of the scale to a thousand times it (a plane, nearly);
//! - **a block and a cylinder** in any attitude, slid toward the body
//!   until the least distance from the body's points to them (closed
//!   form per point: a box's and a cylinder's distance, their depth
//!   inside) is the gap: the least over real points of the body, so no
//!   less than the true distance, and a point of the body inside the
//!   other is a shared volume.
//!
//! The oracles, none of which uses the proof:
//! - `clear`, or a proven `atLeast`, is never said of bodies that share
//!   volume deeper than the tolerance;
//! - `atLeast` is never above the distance between two real points of
//!   the bodies (the ball's closed form; the sampled least for the
//!   others), of the clearance and of the distance, in either order;
//! - against the ball, whose truth is exact: an overlap is never said of
//!   bodies apart by more than the tolerance, a touch never of bodies
//!   more than four tolerances apart or sunk deeper than two, and the
//!   value found is never below the truth.
//!
//! The thickness: a floor under a pit that comes to a point (a drill
//! point), to a pole (a ball-nosed cutter) and to a spline's tip, whose
//! least line into the material is the one from the floor's underside
//! straight up into the tip: `atLeast` is at most the kernel's own ray
//! there (`measure.ray`, which takes no part in the proof) and at most
//! the closed form.
//!
//! The gate runs a fixed set per shape; `-- --ignored` runs the wide
//! sweep (`HUNT_CASES` cases per shape and partner, default 60).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod hunt_checks_common;

use craft3d_math::Portable;
use hunt_checks_common::*;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type P2 = [f64; 2];

/// A piece of a profile in the half-plane `(ρ, z)`.
#[derive(Clone, Copy, Debug)]
enum Seg {
    Line(P2, P2),
    /// About `c` at radius `r` from the angle `a0` to `a1` (clockwise
    /// when `a1 < a0`).
    Arc {
        c: P2,
        r: f64,
        a0: f64,
        a1: f64,
    },
    Curve(Bezier),
}

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn len(a: P2) -> f64 {
    a[0].phypot(a[1])
}

impl Seg {
    fn at(&self, t: f64) -> P2 {
        match self {
            Seg::Line(a, b) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            Seg::Arc { c, r, a0, a1 } => {
                let a = a0 + (a1 - a0) * t;
                [c[0] + r * a.pcos(), c[1] + r * a.psin()]
            }
            Seg::Curve(b) => b.at(t),
        }
    }

    /// The unit direction it runs in at `t`.
    fn tangent(&self, t: f64) -> P2 {
        let d = match self {
            Seg::Line(a, b) => sub(*b, *a),
            Seg::Arc { a0, a1, .. } => {
                let a = a0 + (a1 - a0) * t;
                let s = if a1 >= a0 { 1.0 } else { -1.0 };
                [-a.psin() * s, a.pcos() * s]
            }
            Seg::Curve(b) => {
                // At an end whose control points coincide the derivative
                // is 0: a little way in.
                let d = b.d(t);
                if len(d) > 0.0 {
                    d
                } else {
                    b.d(t.clamp(1e-6, 1.0 - 1e-6))
                }
            }
        };
        let l = len(d);
        [d[0] / l, d[1] / l]
    }

    /// Outward of a profile that runs with its inside on the left.
    fn outward(&self, t: f64) -> P2 {
        let d = self.tangent(t);
        [d[1], -d[0]]
    }

    /// The least distance from `q`.
    fn distance(&self, q: P2) -> f64 {
        match self {
            Seg::Line(a, b) => {
                let (qa, ab) = (sub(q, *a), sub(*b, *a));
                let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if l2 > 0.0 {
                    ((qa[0] * ab[0] + qa[1] * ab[1]) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                len([qa[0] - t * ab[0], qa[1] - t * ab[1]])
            }
            Seg::Arc { c, r, a0, a1 } => {
                let v = sub(q, *c);
                let (lo, hi) = (a0.min(*a1), a0.max(*a1));
                let angle = v[1].patan2(v[0]);
                let turns = ((lo - angle) / TAU).ceil();
                let ends = len(sub(q, self.at(0.0))).min(len(sub(q, self.at(1.0))));
                if angle + turns * TAU <= hi {
                    (len(v) - r).abs().min(ends)
                } else {
                    ends
                }
            }
            Seg::Curve(b) => b.nearest(q).0,
        }
    }

    /// A line on the axis: no face.
    fn on_axis(&self) -> bool {
        matches!(self, Seg::Line(a, b) if a[0] == 0.0 && b[0] == 0.0)
    }
}

/// What a body is, beside its profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `feature.cone`: one face round its apex, no edge there.
    Cone,
    /// A triangle with its tip on the axis, turned all the way.
    RevolvedCone,
    /// Two cones base to base: two apexes.
    Bicone,
    /// A cone cut short: no apex in the face.
    Frustum,
    /// A sphere's cap through its pole, on a disc.
    Dome,
    /// A half circle on the axis: a sphere without edges, both poles.
    Ball,
    /// An arc that ends on the axis off its circle's centre: a pointed
    /// tip on a rational spline surface.
    Ogive,
    /// A cubic that ends on the axis square to it: a blunt tip.
    BluntTip,
    /// A cubic that ends on the axis at a slant: a pointed tip.
    SharpTip,
    /// `feature.torus`: no edge at all.
    Torus,
    /// A cone turned part of the way: its apex is a vertex.
    PartCone,
    /// A dome turned part of the way: its pole is a vertex.
    PartDome,
    /// A disc with a pit that comes to a point (a drill point).
    ConePit,
    /// A disc with a pit that ends in a sphere's pole (a ball nose).
    BallPit,
}

const SHAPES: [Shape; 14] = [
    Shape::Cone,
    Shape::RevolvedCone,
    Shape::Bicone,
    Shape::Frustum,
    Shape::Dome,
    Shape::Ball,
    Shape::Ogive,
    Shape::BluntTip,
    Shape::SharpTip,
    Shape::Torus,
    Shape::PartCone,
    Shape::PartDome,
    Shape::ConePit,
    Shape::BallPit,
];

/// What the body is measured against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Partner {
    Ball,
    Block,
    Cylinder,
}

const PARTNERS: [Partner; 3] = [Partner::Ball, Partner::Block, Partner::Cylinder];

/// A body: the steps that make it as `A0` about the y axis, its profile
/// (inside on the left), the part of a turn it spans about the sketch's
/// plane (π: all of it) and its size.
struct Body {
    steps: Vec<Step>,
    profile: Vec<Seg>,
    half_turn: f64,
    size: f64,
    what: String,
}

fn xy(p: P2) -> Value {
    json!([p[0], p[1]])
}

/// The sketch of a profile of lines and arcs and its turn about y.
fn turned(profile: &[Seg], degrees: Option<f64>) -> Vec<Step> {
    let start = profile[0].at(0.0);
    let last = profile.len() - 1;
    let segments: Vec<Value> = profile
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let end = if i == last {
                json!("start")
            } else {
                xy(seg.at(1.0))
            };
            match seg {
                Seg::Line(..) => json!({"line": end}),
                Seg::Arc { c, a0, a1, .. } => {
                    json!({"arc": end, "center": xy(*c), "clockwise": a1 < a0})
                }
                Seg::Curve(_) => panic!("a cubic is sketched as a spline"),
            }
        })
        .collect();
    let mut revolve = json!({"sketch": "PV", "axis": "y", "name": "A0"});
    if let Some(d) = degrees {
        revolve["angle"] = json!(format!("{d:?} deg"));
        revolve["symmetric"] = json!(true);
    }
    vec![
        ("sketch.create", json!({"name": "PV", "plane": "xy"})),
        (
            "sketch.path",
            json!({"sketch": "PV", "start": xy(start), "segments": segments}),
        ),
        ("feature.revolve", revolve),
    ]
}

fn line(a: P2, b: P2) -> Seg {
    Seg::Line(a, b)
}

/// One body of `shape` at scale `k`.
fn body(rng: &mut Rng, shape: Shape, k: f64) -> Body {
    let r = rng.between(0.3, 5.0) * k;
    let h = rng.between(1.0, 10.0) * k;
    let whole = PI;
    let o = [0.0, 0.0];
    match shape {
        Shape::Cone => Body {
            steps: vec![(
                "feature.cone",
                json!({"name": "A0", "radius": mm(r), "height": mm(h), "axis": [0, 1, 0]}),
            )],
            profile: vec![
                line(o, [r, 0.0]),
                line([r, 0.0], [0.0, h]),
                line([0.0, h], o),
            ],
            half_turn: whole,
            size: r.max(h),
            what: format!("a cone {r:.6e}, {h:.6e}"),
        },
        Shape::RevolvedCone | Shape::PartCone => {
            let profile = vec![
                line(o, [r, 0.0]),
                line([r, 0.0], [0.0, h]),
                line([0.0, h], o),
            ];
            let part = (shape == Shape::PartCone).then(|| rng.between(60.0, 300.0));
            Body {
                steps: turned(&profile, part),
                profile,
                half_turn: part.map_or(whole, |d| 0.5 * d.to_radians()),
                size: r.max(h),
                what: format!("a turned triangle {r:.6e}, {h:.6e}, {part:?} deg"),
            }
        }
        Shape::Bicone => {
            let h2 = rng.between(1.0, 10.0) * k;
            let profile = vec![
                line([0.0, -h2], [r, 0.0]),
                line([r, 0.0], [0.0, h]),
                line([0.0, h], [0.0, -h2]),
            ];
            Body {
                steps: turned(&profile, None),
                profile,
                half_turn: whole,
                size: r.max(h + h2),
                what: format!("two cones {r:.6e}, {h:.6e} and {h2:.6e}"),
            }
        }
        Shape::Frustum => {
            let r2 = rng.between(0.1, 0.9) * r;
            let profile = vec![
                line(o, [r, 0.0]),
                line([r, 0.0], [r2, h]),
                line([r2, h], [0.0, h]),
                line([0.0, h], o),
            ];
            Body {
                steps: turned(&profile, None),
                profile,
                half_turn: whole,
                size: r.max(h),
                what: format!("a frustum {r:.6e} to {r2:.6e}, {h:.6e}"),
            }
        }
        Shape::Dome | Shape::PartDome => {
            // A sphere of radius `s` about (0, -c), cut at z = 0.
            let s = rng.between(1.0, 6.0) * k;
            let c = rng.between(-0.6, 0.8) * s;
            let a = (s * s - c * c).sqrt();
            let a0 = c.patan2(a);
            let profile = vec![
                line(o, [a, 0.0]),
                Seg::Arc {
                    c: [0.0, -c],
                    r: s,
                    a0,
                    a1: FRAC_PI_2,
                },
                line([0.0, s - c], o),
            ];
            let part = (shape == Shape::PartDome).then(|| rng.between(60.0, 300.0));
            Body {
                steps: turned(&profile, part),
                profile,
                half_turn: part.map_or(whole, |d| 0.5 * d.to_radians()),
                size: s,
                what: format!("a dome of radius {s:.6e}, its centre {c:.6e} under, {part:?} deg"),
            }
        }
        Shape::Ball => {
            let profile = vec![
                Seg::Arc {
                    c: o,
                    r,
                    a0: -FRAC_PI_2,
                    a1: FRAC_PI_2,
                },
                line([0.0, r], [0.0, -r]),
            ];
            Body {
                steps: turned(&profile, None),
                profile,
                half_turn: whole,
                size: r,
                what: format!("a turned half circle {r:.6e}"),
            }
        }
        Shape::Ogive => {
            // An arc about (-m, 0) from (r, 0) up to the axis.
            let m = rng.between(0.2, 3.0) * r;
            let s = r + m;
            let top = (s * s - m * m).sqrt();
            let profile = vec![
                line(o, [r, 0.0]),
                Seg::Arc {
                    c: [-m, 0.0],
                    r: s,
                    a0: 0.0,
                    a1: top.patan2(m),
                },
                line([0.0, top], o),
            ];
            Body {
                steps: turned(&profile, None),
                profile,
                half_turn: whole,
                size: r.max(top),
                what: format!("an ogive {r:.6e} wide, its arc's centre {m:.6e} beyond the axis"),
            }
        }
        Shape::BluntTip | Shape::SharpTip => {
            let c1 = [r * rng.between(0.8, 1.2), h * rng.between(0.3, 0.6)];
            let c2 = if shape == Shape::BluntTip {
                [r * rng.between(0.3, 0.8), h]
            } else {
                [r * rng.between(0.1, 0.4), h * rng.between(0.6, 0.85)]
            };
            let curve = Bezier([[r, 0.0], c1, c2, [0.0, h]]);
            let steps = vec![
                ("sketch.create", json!({"name": "PV", "plane": "xy"})),
                (
                    "sketch.polyline",
                    json!({"sketch": "PV", "points": [[0.0, h], [0.0, 0.0], [r, 0.0]]}),
                ),
                (
                    "sketch.spline",
                    json!({"sketch": "PV", "control": ["$last", xy(c1), xy(c2), "$first"]}),
                ),
                (
                    "feature.revolve",
                    json!({"sketch": "PV", "axis": "y", "name": "A0"}),
                ),
            ];
            Body {
                steps,
                profile: vec![line(o, [r, 0.0]), Seg::Curve(curve), line([0.0, h], o)],
                half_turn: whole,
                size: r.max(h) * 1.2,
                what: format!(
                    "a turned cubic {r:.6e}, {h:.6e} through ({:.4e}, {:.4e}), ({:.4e}, {:.4e})",
                    c1[0], c1[1], c2[0], c2[1]
                ),
            }
        }
        Shape::Torus => {
            let major = rng.between(2.0, 8.0) * k;
            let minor = rng.between(0.15, 0.7) * major;
            Body {
                steps: vec![(
                    "feature.torus",
                    json!({"name": "A0", "major": mm(major), "minor": mm(minor), "axis": [0, 1, 0]}),
                )],
                profile: vec![Seg::Arc {
                    c: [major, 0.0],
                    r: minor,
                    a0: 0.0,
                    a1: TAU,
                }],
                half_turn: whole,
                size: major + minor,
                what: format!("a ring {major:.6e}, {minor:.6e}"),
            }
        }
        Shape::ConePit | Shape::BallPit => {
            // A disc of radius `w` and height `h` with a pit of radius
            // `p` at its mouth, `d` deep, from its top.
            let p = rng.between(0.5, 3.0) * k;
            let w = p + rng.between(1.0, 4.0) * k;
            let d = rng.between(0.3, 0.9) * h.min(4.0 * p);
            let pit = if shape == Shape::ConePit {
                line([p, h], [0.0, h - d])
            } else {
                // A sphere's cap `d.min(p)` deep under a mouth of radius
                // `p`: its radius from the chord and the sagitta.
                let d = d.min(p);
                let s = (p * p + d * d) / (2.0 * d);
                let centre = [0.0, h - d + s];
                Seg::Arc {
                    c: centre,
                    r: s,
                    a0: (h - centre[1]).patan2(p),
                    a1: -FRAC_PI_2,
                }
            };
            let bottom = pit.at(1.0);
            let profile = vec![
                line(o, [w, 0.0]),
                line([w, 0.0], [w, h]),
                line([w, h], [p, h]),
                pit,
                line(bottom, o),
            ];
            Body {
                steps: turned(&profile, None),
                profile,
                half_turn: whole,
                size: w.max(h),
                what: format!(
                    "a disc {w:.6e}, {h:.6e} with a pit {p:.6e} wide to {:.6e} over its floor",
                    bottom[1]
                ),
            }
        }
    }
}

/// The point of the body at `p` of its profile turned by `phi` about y.
fn turn(p: P2, phi: f64) -> V {
    V::new(p[0] * phi.pcos(), p[1], p[0] * phi.psin())
}

/// How far `q` (in the half-plane) is from the profile's faces.
fn from_profile(profile: &[Seg], q: P2) -> f64 {
    profile
        .iter()
        .filter(|s| !s.on_axis())
        .map(|s| s.distance(q))
        .fold(f64::INFINITY, f64::min)
}

/// Whether `q` is inside the profile (crossings of a ray along +ρ with
/// the profile drawn as a polygon).
fn inside(profile: &[Seg], q: P2) -> bool {
    let mut points: Vec<P2> = Vec::new();
    for seg in profile {
        let n = if matches!(seg, Seg::Line(..)) { 1 } else { 256 };
        for i in 0..n {
            points.push(seg.at(i as f64 / n as f64));
        }
    }
    let mut odd = false;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        if (a[1] > q[1]) != (b[1] > q[1]) {
            let x = a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if x > q[0] {
                odd = !odd;
            }
        }
    }
    odd
}

/// A place on the profile's faces and a direction out of the body there:
/// along a piece (its outward normal) or at a corner or a tip that
/// sticks out (a direction between its two sides' normals; at the axis,
/// between the side's normal and the axis).
fn place(rng: &mut Rng, profile: &[Seg]) -> (P2, P2) {
    let faces: Vec<usize> = (0..profile.len())
        .filter(|i| !profile[*i].on_axis())
        .collect();
    let i = rng.pick(&faces);
    let seg = &profile[i];
    if rng.below(5) < 2 {
        let t = rng.between(0.05, 0.95);
        return (seg.at(t), seg.outward(t));
    }
    // An end: this piece's end and the next one's start.
    let next = &profile[(i + 1) % profile.len()];
    let (p, n) = (seg.at(1.0), seg.outward(1.0));
    let m = if next.on_axis() {
        // Round the axis the other side's normal is this one's mirrored.
        [-n[0], n[1]]
    } else {
        next.outward(0.0)
    };
    let (t0, t1) = (seg.tangent(1.0), next.tangent(0.0));
    let sticks_out = if next.on_axis() {
        n[0] > 0.0
    } else {
        t0[0] * t1[1] - t0[1] * t1[0] > 0.0
    };
    if !sticks_out {
        let t = rng.between(0.9, 0.999);
        return (seg.at(t), seg.outward(t));
    }
    // Round the axis: only the half that stays in the half-plane.
    let s = if next.on_axis() {
        rng.between(0.0, 0.5)
    } else {
        rng.unit()
    };
    let d = [n[0] + (m[0] - n[0]) * s, n[1] + (m[1] - n[1]) * s];
    let l = len(d);
    if l < 1e-9 {
        return (p, n);
    }
    (p, [d[0] / l, d[1] / l])
}

/// The truth of a case.
#[derive(Clone, Copy, Debug)]
struct Truth {
    /// No more than this apart (the distance of two real points); for a
    /// ball, the distance itself.
    gap: f64,
    /// At least this deep in each other (0 when not).
    depth: f64,
    /// `gap` is the distance itself, not only above it.
    exact: bool,
}

struct Case {
    id: String,
    what: String,
    k: f64,
    steps: Vec<Step>,
    truth: Truth,
    motion: Motion,
}

const SCALES: [f64; 4] = [1e-3, 1.0, 1.0, 1e6];

/// Gaps as fractions of the scale, as the hunts of the distance take
/// them, down to sunk in a twentieth.
const GAPS: [f64; 16] = [
    3.0, 0.3, 1e-2, 1e-4, 1e-5, 2e-6, 1e-6, 1e-7, 1e-9, 0.0, -1e-9, -1e-7, -1e-6, -3e-6, -1e-4,
    -0.05,
];

/// A convex partner: its signed distance from a point in its own frame
/// (negative inside: the depth).
#[derive(Clone, Copy, Debug)]
enum Solid {
    Block(V),
    Cylinder(f64, f64),
}

impl Solid {
    fn signed(&self, p: V) -> f64 {
        match *self {
            Solid::Block(size) => {
                let lo = V::new(0.0, 0.0, 0.0);
                let d = box_distance(lo, size, p);
                if d > 0.0 { d } else { -box_depth(lo, size, p) }
            }
            Solid::Cylinder(r, h) => {
                let d = cylinder_distance(r, h, p);
                if d > 0.0 {
                    d
                } else {
                    -(r - p.x.phypot(p.y)).min(p.z).min(h - p.z)
                }
            }
        }
    }
}

/// The least signed distance from the body's points (its profile's
/// pieces at `nt` places each, turned to `nphi` angles within its turn)
/// to `solid` placed by `pose`, and where it is least (piece, t, angle).
fn least_from(
    body: &Body,
    solid: &Solid,
    pose: &Motion,
    nt: usize,
    nphi: usize,
    around: Option<(usize, f64, f64)>,
) -> (f64, (usize, f64, f64)) {
    let back = Turn {
        axis: pose.turn.axis,
        deg: -pose.turn.deg,
    };
    let mut best = (f64::INFINITY, (0, 0.0, 0.0));
    for (i, seg) in body.profile.iter().enumerate() {
        if seg.on_axis() || around.is_some_and(|(j, ..)| j != i) {
            continue;
        }
        // Near the last least place, or all of the piece and the turn.
        let (t0, t1, p0, p1) = match around {
            Some((_, t, phi)) => {
                let (dt, dp) = (1.5 / nt as f64, 1.5 * body.half_turn / nphi as f64);
                (
                    (t - dt).max(0.0),
                    (t + dt).min(1.0),
                    (phi - dp).max(-body.half_turn),
                    (phi + dp).min(body.half_turn),
                )
            }
            None => (0.0, 1.0, -body.half_turn, body.half_turn),
        };
        let fine = if around.is_some() { 120 } else { nt };
        let fine_phi = if around.is_some() { 120 } else { nphi };
        for a in 0..=fine {
            let t = t0 + (t1 - t0) * a as f64 / fine as f64;
            let p = seg.at(t);
            for b in 0..=fine_phi {
                let phi = p0 + (p1 - p0) * b as f64 / fine_phi as f64;
                let d = solid.signed(back.apply(turn(p, phi) - pose.shift));
                if d < best.0 {
                    best = (d, (i, t, phi));
                }
            }
        }
    }
    best
}

/// One case: a body of `shape` and a partner at a gap drawn from `rng`.
/// `None` where the draw makes no case (the generator draws again).
fn case(rng: &mut Rng, shape: Shape, partner: Partner, index: usize) -> Option<Case> {
    let k = rng.pick(&SCALES);
    let g = rng.pick(&GAPS) * k;
    let motion = Motion::random(rng, k);
    let body = body(rng, shape, k);
    let (p, dir) = place(rng, &body.profile);
    let mut steps = body.steps.clone();
    let (truth, with) = match partner {
        Partner::Ball => {
            let rb = match rng.below(3) {
                0 => rng.between(0.3, 3.0),
                1 => rng.between(3.0, 30.0),
                _ => rng.between(30.0, 1000.0),
            } * k;
            if rb + g <= 0.05 * k {
                return None;
            }
            let mut q = [p[0] + dir[0] * (rb + g), p[1] + dir[1] * (rb + g)];
            // Across the axis is the same body seen from the other side.
            q[0] = q[0].abs();
            if inside(&body.profile, q) {
                return None;
            }
            let gap = from_profile(&body.profile, q) - rb;
            // In a pit a ball may meet the other wall first: only cases
            // near the gap drawn.
            if (gap - g).abs() > 1e-6 * (rb + body.size) && shape != Shape::Torus {
                return None;
            }
            steps.push((
                "feature.sphere",
                json!({"name": "B0", "radius": mm(rb), "position": mm3(turn(q, 0.0))}),
            ));
            (
                Truth {
                    gap: gap.max(0.0),
                    depth: (-gap).max(0.0),
                    exact: true,
                },
                format!("a ball {rb:.6e} at ({:.6e}, {:.6e})", q[0], q[1]),
            )
        }
        Partner::Block | Partner::Cylinder => {
            let (solid, make): (Solid, Step) = if partner == Partner::Block {
                let size = V::new(
                    rng.between(1.0, 12.0) * k,
                    rng.between(1.0, 12.0) * k,
                    rng.between(1.0, 12.0) * k,
                );
                (
                    Solid::Block(size),
                    ("feature.block", json!({"name": "B1", "size": mm3(size)})),
                )
            } else {
                let (r, h) = (rng.between(0.5, 6.0) * k, rng.between(1.0, 12.0) * k);
                (
                    Solid::Cylinder(r, h),
                    (
                        "feature.cylinder",
                        json!({"name": "B1", "radius": mm(r), "height": mm(h)}),
                    ),
                )
            };
            // In any attitude, its middle on the line out of the body at
            // the place along `dir`: from far off, slid in until the
            // body's points are `g` from it.
            let turned = Turn::random(rng);
            let phi = rng.between(-0.9, 0.9) * body.half_turn;
            let (from, along) = (turn(p, phi), turn(dir, phi));
            let middle = turned.apply(match solid {
                Solid::Block(size) => size * 0.5,
                Solid::Cylinder(_, h) => V::new(0.0, 0.0, 0.5 * h),
            });
            let far = 40.0 * k + body.size;
            let pose = |s: f64| Motion {
                turn: turned,
                shift: from + along * s - middle,
            };
            let least = |s: f64| least_from(&body, &solid, &pose(s), 24, 48, None).0;
            if least(far) <= g {
                return None;
            }
            // With its middle at the place it is sunk in.
            let (mut lo, mut hi) = (0.0, far);
            if least(lo) > g {
                return None;
            }
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if least(mid) > g {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let pose = pose(hi);
            // Looked at closely: all of it, then round the least place.
            let (_, at) = least_from(&body, &solid, &pose, 96, 192, None);
            let (d, at) = least_from(&body, &solid, &pose, 96, 192, Some(at));
            let (d2, _) = least_from(&body, &solid, &pose, 96 * 80, 192 * 80, Some(at));
            let d = d.min(d2);
            steps.push(make);
            steps.push(pose.step("B1", "B0"));
            (
                Truth {
                    gap: d.max(0.0),
                    depth: (-d).max(0.0),
                    exact: false,
                },
                format!("{solid:?} slid to {d:.6e} of it"),
            )
        }
    };
    steps.push(motion.step("A0", "A"));
    steps.push(motion.step("B0", "B"));
    Some(Case {
        id: format!("{shape:?} {partner:?} {index}"),
        what: format!(
            "{} and {with}, scale {k:e}, drawn gap {g:e}, {}",
            body.what,
            motion.describe()
        ),
        k,
        steps,
        truth,
        motion,
    })
}

/// The steps run, a spline's `$last` and `$first` being the ends of the
/// polyline sketched before it.
fn build_case(s: &mut craft3d_engine::Session, steps: &[Step]) -> Result<(), Ran> {
    let mut points: Vec<Value> = Vec::new();
    for (id, params) in steps {
        let mut params = params.clone();
        if *id == "sketch.spline" {
            for c in params["control"].as_array_mut().unwrap().iter_mut() {
                if c == "$last" {
                    *c = points.last().cloned().unwrap_or(Value::Null);
                } else if c == "$first" {
                    *c = points.first().cloned().unwrap_or(Value::Null);
                }
            }
        }
        match run(s, id, &params) {
            Ran::Ok(v) => {
                if *id == "sketch.polyline" {
                    points = v["points"].as_array().cloned().unwrap_or_default();
                }
            }
            other => return Err(other),
        }
    }
    Ok(())
}

/// What was proven of a case, for the count of how close the proofs
/// come.
#[derive(Default)]
struct Proofs {
    /// Clearances answered clear.
    clear: usize,
    /// Of them, proven to within two tolerances of the truth (a ball's).
    tight: usize,
    /// The largest share of the gap left unproven (a ball's).
    short: f64,
}

/// The verdict of a case and what was wrong; `None` when the engine did
/// not build it.
fn judge(case: &Case, proofs: &mut Proofs) -> Option<(Verdict, String)> {
    let mut s = document(case.k);
    match build_case(&mut s, &case.steps) {
        Ok(()) => {}
        Err(Ran::Panic(p)) => return Some((Verdict::Panic, format!("building: {p}"))),
        Err(_) => return None,
    }
    let tol = 1e-6 * case.k;
    let t = case.truth;
    // The rounding of the bodies' own coordinates, far from the origin.
    let eps = 1e-13 * (case.motion.reach() + 1100.0 * case.k) + 1e-12 * t.gap;
    let (mut wrong, mut declined, mut panicked) = (Vec::new(), Vec::new(), Vec::new());
    for (a, b) in [("A", "B"), ("B", "A")] {
        let tag = format!("clearance {a} {b}");
        match run(&mut s, "measure.clearance", &json!({"from": a, "to": b})) {
            Ran::Panic(p) => panicked.push(format!("{tag}: {p}")),
            Ran::Err(e) => declined.push(format!("{tag}: {e}")),
            Ran::Ok(out) => {
                let status = out["pairs"][0]["status"].as_str().unwrap_or("?");
                let (value, at_least) = (out["clearance"].as_f64(), out["atLeast"].as_f64());
                match status {
                    "overlap" => {
                        if t.exact && t.depth == 0.0 && t.gap > tol {
                            wrong
                                .push(format!("{tag}: overlap said of bodies {:.6e} apart", t.gap));
                        }
                        if value.is_some() || at_least.is_some() {
                            wrong.push(format!("{tag}: overlap with a value {out}"));
                        }
                    }
                    "touch" => {
                        if t.depth > 2.0 * tol {
                            wrong.push(format!(
                                "{tag}: touch said of bodies sharing volume {:.6e} deep",
                                t.depth
                            ));
                        }
                        if t.exact && t.depth == 0.0 && t.gap > 4.0 * tol {
                            wrong.push(format!("{tag}: touch said of bodies {:.6e} apart", t.gap));
                        }
                    }
                    "clear" => {
                        // (A depth drawn at the tolerance itself comes out
                        // of the closed form a rounding above it.)
                        if t.depth > tol * (1.0 + 1e-6) + eps {
                            wrong.push(format!(
                                "{tag}: clear ({value:?}, atLeast {at_least:?}) said of bodies sharing volume {:.6e} deep",
                                t.depth
                            ));
                        }
                        match (value, at_least) {
                            (Some(v), Some(l)) => {
                                if l > t.gap + eps {
                                    wrong.push(format!(
                                        "{tag}: atLeast {l:.17e} above the distance of two real points {:.17e} (by {:.3e})",
                                        t.gap,
                                        l - t.gap
                                    ));
                                }
                                if l > v {
                                    wrong.push(format!(
                                        "{tag}: atLeast {l:e} above the value {v:e}"
                                    ));
                                }
                                if t.exact && v < t.gap - 2.0 * tol - eps {
                                    wrong.push(format!(
                                        "{tag}: clearance {v:.17e} below the true distance {:.17e}",
                                        t.gap
                                    ));
                                }
                                if t.exact && t.depth == 0.0 {
                                    proofs.clear += 1;
                                    if l >= t.gap - 2.0 * tol - eps {
                                        proofs.tight += 1;
                                    } else if t.gap > 0.0 {
                                        proofs.short = proofs.short.max((t.gap - l) / t.gap);
                                    }
                                }
                            }
                            _ => wrong.push(format!("{tag}: clear without a value {out}")),
                        }
                    }
                    other => declined.push(format!("{tag}: status {other}")),
                }
            }
        }
    }
    match run(&mut s, "measure.distance", &json!({"from": "A", "to": "B"})) {
        Ran::Panic(p) => panicked.push(format!("distance: {p}")),
        Ran::Err(e) => declined.push(format!("distance: {e}")),
        Ran::Ok(out) => match (out["distance"].as_f64(), out["atLeast"].as_f64()) {
            (Some(d), Some(l)) => {
                // Sunk in, the faces cross: 0 apart.
                if l > t.gap + eps {
                    wrong.push(format!(
                        "distance: atLeast {l:.17e} above the distance of two real points of the faces {:.17e} (by {:.3e})",
                        t.gap,
                        l - t.gap
                    ));
                }
                if t.exact && d < t.gap - 2.0 * tol - eps {
                    wrong.push(format!(
                        "distance: {d:.17e} below the true distance of the faces {:.17e}",
                        t.gap
                    ));
                }
            }
            _ => declined.push(format!("distance: no distance in {out}")),
        },
    }
    let verdict = if !panicked.is_empty() {
        Verdict::Panic
    } else if !wrong.is_empty() {
        Verdict::Wrong
    } else if !declined.is_empty() {
        Verdict::Declined
    } else {
        Verdict::Right
    };
    let mut lines = panicked;
    lines.extend(wrong);
    lines.extend(declined);
    Some((verdict, lines.join("; ")))
}

const SEED: u64 = 0x5EED_A9E0_0001;

/// `count` cases of `shape` against `partner` that the engine builds.
fn sweep(shape: Shape, partner: Partner, count: usize, proofs: &mut Proofs) -> Tally {
    let salt = (shape as u64 * 16 + partner as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut rng = Rng(SEED ^ salt);
    let mut tally = Tally::default();
    let (mut index, mut draws) = (0, 0);
    while index < count && draws < 40 * count {
        draws += 1;
        let Some(case) = case(&mut rng, shape, partner, index) else {
            continue;
        };
        let Some((verdict, why)) = judge(&case, proofs) else {
            continue;
        };
        let line = format!("{}: {why}", case.what);
        if verdict != Verdict::Right {
            println!("{} {} {line}", verdict.letter(), case.id);
        }
        tally.add(&case.id, verdict, line);
        index += 1;
    }
    tally
}

/// Every case of the shapes against every partner is right or declined
/// with a reason: none wrong, none panicking; and most are answered.
fn hold(shapes: &[Shape], count: usize) {
    let mut all = Tally::default();
    let mut proofs = Proofs::default();
    for &shape in shapes {
        for partner in PARTNERS {
            let tally = sweep(shape, partner, count, &mut proofs);
            println!("{}", tally.summary(&format!("{shape:?} {partner:?}")));
            all.merge(tally);
        }
    }
    all.print("all");
    println!(
        "clear against a ball: {}, proven to within two tolerances: {}, the most of a gap left unproven: {:.3e}",
        proofs.clear, proofs.tight, proofs.short
    );
    assert!(all.cases >= shapes.len() * count, "{} cases", all.cases);
    assert_eq!(all.wrong + all.panic, 0, "wrong or panicking cases");
}

const GATE_CASES: usize = 6;

#[test]
fn cones_with_and_without_their_apex() {
    hold(
        &[
            Shape::Cone,
            Shape::RevolvedCone,
            Shape::Bicone,
            Shape::Frustum,
            Shape::PartCone,
        ],
        GATE_CASES,
    );
}

#[test]
fn spheres_through_their_poles_and_rings() {
    hold(
        &[Shape::Dome, Shape::Ball, Shape::PartDome, Shape::Torus],
        GATE_CASES,
    );
}

#[test]
fn tips_of_arcs_and_splines_turned_about_their_end() {
    hold(
        &[Shape::Ogive, Shape::BluntTip, Shape::SharpTip],
        GATE_CASES,
    );
}

#[test]
fn pits_that_come_to_a_point_or_a_pole() {
    hold(&[Shape::ConePit, Shape::BallPit], GATE_CASES);
}

/// The wide sweep's cases per shape and partner.
fn wide_cases() -> usize {
    std::env::var("HUNT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
}

#[test]
#[ignore = "the wide sweep: cargo test -p craft3d-engine --test proofs_at_poles -- --ignored"]
fn wide_cones() {
    hold(&SHAPES[..4], wide_cases());
    hold(&[Shape::PartCone], wide_cases());
}

#[test]
#[ignore = "the wide sweep"]
fn wide_spheres_and_rings() {
    hold(
        &[Shape::Dome, Shape::Ball, Shape::PartDome, Shape::Torus],
        wide_cases(),
    );
}

#[test]
#[ignore = "the wide sweep"]
fn wide_tips() {
    hold(
        &[Shape::Ogive, Shape::BluntTip, Shape::SharpTip],
        wide_cases(),
    );
}

#[test]
#[ignore = "the wide sweep"]
fn wide_pits() {
    hold(&[Shape::ConePit, Shape::BallPit], wide_cases());
}

// ------------------------------------------------------------ thickness

/// The floor under a pit: a block with a pit from its top whose lowest
/// point is `floor` over its underside, as the steps that make `A` and
/// the place under the tip.
fn pitted(kind: usize, k: f64) -> (Vec<Step>, V, f64, &'static str) {
    let (r, floor, depth) = (2.0 * k, 0.5 * k, 6.0 * k);
    let size = V::new(14.0 * k, 14.0 * k, depth + floor);
    let mid = V::new(7.0 * k, 7.0 * k, 0.0);
    let top = size.z;
    let blank = ("feature.block", json!({"name": "Blank", "size": mm3(size)}));
    let cut = |tool: &str| {
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Blank", "tool": tool, "name": "A0"}),
        )
    };
    let up = V::new(0.0, 0.0, 1.0);
    match kind {
        // A drill point of 118°: a bore and a cone under it.
        0 => {
            let tip = r / 59.0_f64.to_radians().ptan();
            let bore = depth - tip;
            let at = mid + up * (top - bore);
            (
                vec![
                    blank,
                    (
                        "feature.cylinder",
                        json!({"name": "Bore", "radius": mm(r), "height": mm(bore + k), "position": mm3(at)}),
                    ),
                    (
                        "feature.cone",
                        json!({"name": "Point", "radius": mm(r), "height": mm(tip), "position": mm3(at), "axis": [0, 0, -1]}),
                    ),
                    (
                        "feature.boolean",
                        json!({"op": "union", "target": "Bore", "tool": "Point", "name": "Drill"}),
                    ),
                    cut("Drill"),
                ],
                mid,
                floor,
                "a drill point",
            )
        }
        // A cone alone, from the top: a countersink to a point.
        1 => (
            vec![
                blank,
                (
                    "feature.cone",
                    json!({"name": "Point", "radius": mm(2.5 * r), "height": mm(depth), "position": mm3(mid + up * top), "axis": [0, 0, -1]}),
                ),
                cut("Point"),
            ],
            mid,
            floor,
            "a pit that is a cone",
        ),
        // A ball-nosed cutter: a bore and a ball under it.
        _ => {
            let bore = depth - r;
            let at = mid + up * (top - bore);
            (
                vec![
                    blank,
                    (
                        "feature.cylinder",
                        json!({"name": "Bore", "radius": mm(r), "height": mm(bore + k), "position": mm3(at)}),
                    ),
                    (
                        "feature.sphere",
                        json!({"name": "Nose", "radius": mm(r), "position": mm3(at)}),
                    ),
                    (
                        "feature.boolean",
                        json!({"op": "union", "target": "Bore", "tool": "Nose", "name": "Cutter"}),
                    ),
                    cut("Cutter"),
                ],
                mid,
                floor,
                "a ball nose",
            )
        }
    }
}

/// The floor under a drill point, under a pit that is a cone and under
/// a ball nose, 0.5 thick under the tip: the least line into the
/// material is the one from the underside straight up into the tip (the
/// kernel's ray there measures the floor), every other one longer. The
/// thickness proven is never above it, and an assertion of a floor a
/// thousandth thicker does not pass; at three scales, in the world and
/// turned and moved far off. (Before, the proof looked at a cone's face
/// only in a band at its rim: `atLeast` was the thickness found, a ray
/// beside the tip, 3.5e-8 above the floor.)
#[test]
fn the_floor_under_a_pit_is_proven_no_thicker_than_it_is() {
    let mut rng = Rng(SEED ^ 0x7417);
    for k in [1e-3, 1.0, 1e6] {
        for kind in 0..3 {
            for far in [false, true] {
                let motion = if far {
                    Motion {
                        turn: Turn::random(&mut rng),
                        shift: rng.direction() * (1000.0 * k),
                    }
                } else {
                    Motion::NONE
                };
                let (mut steps, under, floor, what) = pitted(kind, k);
                steps.push(motion.step("A0", "A"));
                let label = format!("{what} at scale {k:e}, {}", motion.describe());
                let mut s = document(k);
                build_case(&mut s, &steps).unwrap_or_else(|e| panic!("{label}: {e:?}"));
                // The kernel's own line from under the floor into the tip.
                let from = motion.apply(under - V::new(0.0, 0.0, k));
                let dir = motion.turn.apply(V::new(0.0, 0.0, 1.0));
                let Ran::Ok(ray) = run(
                    &mut s,
                    "measure.ray",
                    &json!({"from": mm3(from), "direction": num3(dir), "of": "A"}),
                ) else {
                    panic!("{label}: the ray");
                };
                let through = ray["hits"][1]["distance"].as_f64().unwrap()
                    - ray["hits"][0]["distance"].as_f64().unwrap();
                let eps = 1e-12 * (motion.reach() + 20.0 * k);
                assert!((through - floor).abs() <= eps, "{label}: the ray {through}");
                let Ran::Ok(out) = run(&mut s, "measure.thickness", &json!({"of": "A"})) else {
                    panic!("{label}: the thickness");
                };
                let (found, at_least) = (
                    out["thickness"].as_f64().unwrap(),
                    out["atLeast"].as_f64().unwrap(),
                );
                println!("{label}: found {found:.12e}, proven {at_least:.12e} of {floor:e}");
                assert!(
                    at_least <= floor + eps && at_least <= through + eps,
                    "{label}: proven {at_least:.17e}, above the floor {floor:.17e}"
                );
                assert!(found >= floor - 2e-6 * k, "{label}: found {found:e}");
                // And proven to it: within a billionth.
                assert!(
                    at_least >= floor * (1.0 - 1e-9) - eps,
                    "{label}: proven only {at_least:.17e} of {floor:.17e}"
                );
                // An assertion of the floor itself passes; of a floor a
                // thousandth thicker, never.
                let mut assert = |name: &str, min: f64| {
                    let Ran::Ok(added) = run(
                        &mut s,
                        "assert.add",
                        &json!({"name": name, "measure": "thickness", "of": "A", "min": mm(min)}),
                    ) else {
                        panic!("{label}: the assertion {name}");
                    };
                    added["status"].as_str().unwrap_or("?").to_string()
                };
                assert_eq!(assert("floor", floor), "passed", "{label}");
                let thicker = assert("thicker", floor * 1.001);
                assert!(
                    thicker == "failed" || thicker == "notProven",
                    "{label}: {thicker}"
                );
            }
        }
    }
}

/// A sharp cone that holds its apex is as thin as one likes at its tip:
/// a line into it square to its side, a hundred-thousandth of its height
/// from the apex, comes out of the other side after `s tan 2α` (the
/// kernel's ray there measures it). The thickness proven is no more than
/// that line, and no more than the tolerance; an assertion of a wall of a
/// thousandth of its radius never passes. (Before, the proof looked at
/// the cone only in a band at its base, and what was proven was the least
/// the samples and the search from them had found.) At three scales, in
/// the world and turned and moved far off.
#[test]
fn a_sharp_cone_is_proven_no_thicker_than_it_is_at_its_tip() {
    let mut rng = Rng(SEED ^ 0x5A49);
    for k in [1e-3, 1.0, 1e6] {
        for far in [false, true] {
            let motion = if far {
                Motion {
                    turn: Turn::random(&mut rng),
                    shift: rng.direction() * (1000.0 * k),
                }
            } else {
                Motion::NONE
            };
            let (r, h) = (1.0 * k, 10.0 * k);
            let label = format!("scale {k:e}, {}", motion.describe());
            let mut s = document(k);
            build_case(
                &mut s,
                &[
                    (
                        "feature.cone",
                        json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                    ),
                    motion.step("A0", "A"),
                ],
            )
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
            // A point of the side `along` from the apex, and the way in.
            let slant = r.phypot(h);
            let along = 1e-5 * h;
            let at = V::new(along * r / slant, 0.0, h - along * h / slant);
            let out = V::new(h / slant, 0.0, r / slant);
            let Ran::Ok(ray) = run(
                &mut s,
                "measure.ray",
                &json!({"from": mm3(motion.apply(at + out * k)), "direction": num3(motion.turn.apply(-out)), "of": "A"}),
            ) else {
                panic!("{label}: the ray");
            };
            let through = ray["hits"][1]["distance"].as_f64().unwrap()
                - ray["hits"][0]["distance"].as_f64().unwrap();
            let chord = along * 2.0 * r * h / (h * h - r * r);
            let eps = 1e-12 * (motion.reach() + 20.0 * k);
            assert!(
                (through - chord).abs() <= 1e-6 * chord + eps,
                "{label}: the ray {through:e}, the closed form {chord:e}"
            );
            let Ran::Ok(thin) = run(&mut s, "measure.thickness", &json!({"of": "A"})) else {
                panic!("{label}: the thickness");
            };
            let at_least = thin["atLeast"].as_f64().unwrap();
            println!(
                "{label}: found {:e}, proven {at_least:e}, a line near the tip {through:e}",
                thin["thickness"].as_f64().unwrap()
            );
            assert!(
                at_least <= through + eps && at_least <= 1e-6 * k,
                "{label}: proven {at_least:e}, a line near the tip is {through:e} long"
            );
            let Ran::Ok(added) = run(
                &mut s,
                "assert.add",
                &json!({"name": "wall", "measure": "thickness", "of": "A", "min": mm(1e-3 * r)}),
            ) else {
                panic!("{label}: the assertion");
            };
            let status = added["status"].as_str().unwrap_or("?");
            assert!(
                status == "failed" || status == "notProven",
                "{label}: {added}"
            );
        }
    }
}
