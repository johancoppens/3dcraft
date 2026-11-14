//! A seeded hunt of what `measure.clearance` and `measure.distance` say
//! of two bodies, against closed forms of the test's own: pairs whose true
//! distance is known (ball and ball, a ball inside a ball, a ball against
//! a box's face, edge or corner, a ball inside a box, a ball against a
//! cylinder's wall, cap or rim, parallel and skew cylinders, a pin in a
//! hole off its axis, a ring (torus) over a plane at any tilt, a ball
//! against a ring, a cone over a plane at any tilt (its base flat, its
//! apex down), two boxes in any attitude and two thin plates through each
//! other, a ball in a closed hollow, a ball against a spline-walled prism
//! and two spline-walled prisms), each turned about a random axis and
//! moved far from the origin, at scales 1e-3 to 1e6, at gaps from three
//! times the size down to a billionth of it, touching, and overlapping by
//! a billionth of the size to half of it.
//!
//! The oracles (docs/design/verification.md › *The queries*, *The proven
//! distance*):
//! - `clear` or a proven `atLeast` is never said of bodies that share
//!   volume (deeper than the tolerance), an overlap never of bodies apart
//!   by more than the tolerance, a touch never of bodies more than four
//!   tolerances apart or overlapping deeper than two;
//! - `atLeast` is never above the true distance (beyond the rounding a far
//!   frame brings: 1e-13 of the distance from the origin);
//! - the distance found is a distance between real points, so never below
//!   the truth by more than the tolerance (a point within it of a face
//!   counts as on it), and for faces of planes, cylinders, cones, spheres
//!   and tori it is exact (the command says so) to twice the tolerance;
//!   on a spline face projection may stop short (documented), so only the
//!   lower side is held there;
//! - the same in either order of the two bodies.
//!
//! The truth: closed forms (a point's distance from a box or a cylinder,
//! a line's from a line, a circle's lowest point, Minkowski sums), the
//! separating axes and the corner–face and edge–edge distances of two
//! boxes, and for the spline walls dense sampling refined by golden
//! sections (each a real point of the curve, the samples a 4000th of the
//! parameter apart). A case the engine does not build is not counted.
//!
//! The gate runs a fixed set per kind; `-- --ignored` runs the wide sweeps
//! (`HUNT_CASES` cases per kind, default 120) and prints every case that is
//! not right. Cases found wrong are pinned in [`KNOWN`] with what is wrong
//! (none at present: the ten a cone's face that holds its apex gave were
//! repaired on 2026-11-14). Known wrong and pinned in tests of their own,
//! which fail when it is mended: the distance of faces that cross at a
//! shallow angle is not 0
//! (`a_big_ball_sunk_into_a_block_is_given_a_distance_known`), and a
//! huge ball sunk a few tolerances into a cone's or a cylinder's side is
//! answered clear
//! (`a_huge_ball_sunk_a_few_tolerances_into_a_side_is_answered_clear_known`).

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
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    BallBall,
    BallInBall,
    BallBox,
    BallInBox,
    BallCylinder,
    CylParallel,
    CylSkew,
    PinHole,
    TorusPlane,
    TorusBall,
    ConePlane,
    BoxBox,
    ThinPlates,
    BallInHollow,
    SplineBall,
    SplineSpline,
    /// A ball by a cone: near its apex (a pole), its rim or its side.
    BallCone,
    /// A pin through a ring's hole, on its axis or off it.
    TorusPin,
    /// A ball of a radius 50 to 5000 times the scale over a block.
    BigBall,
    /// A small ball over a block a thousand times its size.
    Mismatch,
    /// A ball by a spline profile turned about an axis.
    Vase,
    /// Two balls in one body (a pattern), each outside a block, in its
    /// hollow or wholly in its material.
    Lumps,
    /// A ball 10 to 1000 times the scale by a cone's side (slender or
    /// not): the settling stops short there.
    ConeBigBall,
}

const KINDS: [Kind; 23] = [
    Kind::BallBall,
    Kind::BallInBall,
    Kind::BallBox,
    Kind::BallInBox,
    Kind::BallCylinder,
    Kind::CylParallel,
    Kind::CylSkew,
    Kind::PinHole,
    Kind::TorusPlane,
    Kind::TorusBall,
    Kind::ConePlane,
    Kind::BoxBox,
    Kind::ThinPlates,
    Kind::BallInHollow,
    Kind::SplineBall,
    Kind::SplineSpline,
    Kind::BallCone,
    Kind::TorusPin,
    Kind::BigBall,
    Kind::Mismatch,
    Kind::Vase,
    Kind::Lumps,
    Kind::ConeBigBall,
];

/// The scales (rule 3).
const SCALES: [f64; 5] = [1e-3, 1.0, 1.0, 1e3, 1e6];

/// Gaps as fractions of the scale: far, near, at ten tolerances, at the
/// tolerance, under it, touching, and overlapping from a billionth to
/// half the scale.
const GAPS: [f64; 19] = [
    3.0, 0.3, 1e-2, 1e-4, 1e-5, 2e-6, 1e-6, 1e-7, 1e-9, 0.0, -1e-9, -1e-7, -1e-6, -3e-6, -1e-5,
    -1e-3, -0.05, -0.2, -0.5,
];

/// The true relation of the two bodies.
#[derive(Clone, Copy, Debug)]
struct Truth {
    /// The distance between the bodies (0 when they touch or overlap).
    gap: f64,
    /// How deep they overlap (0 when they do not): the least move that
    /// parts them, or the inner one's size when one holds the other.
    depth: f64,
    /// The least distance between their boundaries.
    faces: f64,
}

impl Truth {
    fn apart(g: f64) -> Truth {
        if g >= 0.0 {
            Truth {
                gap: g,
                depth: 0.0,
                faces: g,
            }
        } else {
            Truth {
                gap: 0.0,
                depth: -g,
                faces: 0.0,
            }
        }
    }
}

struct Case {
    id: String,
    what: String,
    k: f64,
    steps: Vec<Step>,
    truth: Truth,
    motion: Motion,
    /// Every face is a plane, cylinder, cone, sphere or torus: the
    /// distance found is exact.
    analytic: bool,
}

/// The motion of a case to `A` and `B`.
fn moved(steps: &mut Vec<Step>, motion: &Motion) {
    steps.push(motion.step("A0", "A"));
    steps.push(motion.step("B0", "B"));
}

fn sphere(name: &str, r: f64, c: V) -> Step {
    (
        "feature.sphere",
        json!({"name": name, "radius": mm(r), "position": mm3(c)}),
    )
}

fn block(name: &str, size: V, at: V) -> Step {
    (
        "feature.block",
        json!({"name": name, "size": mm3(size), "position": mm3(at)}),
    )
}

fn cylinder(name: &str, r: f64, h: f64, at: V, axis: V) -> Step {
    (
        "feature.cylinder",
        json!({"name": name, "radius": mm(r), "height": mm(h), "position": mm3(at), "axis": num3(axis)}),
    )
}

fn subtract(name: &str, target: &str, tool: &str) -> Step {
    (
        "feature.boolean",
        json!({"op": "subtract", "target": target, "tool": tool, "name": name}),
    )
}

/// A ball's centre placed at `p0 + n (r + g)` with `n` in the normal cone
/// of a convex body at `p0`: the ball is `g` from the body.
fn place(p0: V, n: V, r: f64, g: f64) -> V {
    p0 + n * (r + g)
}

/// One case of `kind`: its sizes, configuration, gap and frame drawn from
/// `rng`. `None` where the drawn sizes do not make the configuration (the
/// generator then draws again).
fn case(rng: &mut Rng, kind: Kind, index: usize) -> Option<Case> {
    let k = rng.pick(&SCALES);
    let g = rng.pick(&GAPS) * k;
    let motion = Motion::random(rng, k);
    let mut steps: Vec<Step> = Vec::new();
    let mut analytic = true;
    let (truth, what) = match kind {
        Kind::BallBall => {
            let (r1, r2) = (rng.between(2.0, 6.0) * k, rng.between(1.0, 5.0) * k);
            let c1 = rng.direction() * (rng.between(0.0, 3.0) * k);
            let c2 = c1 + rng.direction() * (r1 + r2 + g);
            steps.push(sphere("A0", r1, c1));
            steps.push(sphere("B0", r2, c2));
            (Truth::apart(g), format!("balls {r1:.6e} and {r2:.6e}"))
        }
        Kind::BallInBall => {
            let big = rng.between(4.0, 8.0) * k;
            let r = rng.between(0.5, 0.4 * big / k) * k;
            // g > 0: inside, g from the wall; g < 0: through the wall.
            let e = big - r - g;
            if e < 0.0 {
                return None;
            }
            let c = rng.direction() * (rng.between(-2.0, 2.0) * k);
            steps.push(sphere("A0", big, c));
            steps.push(sphere("B0", r, c + rng.direction() * e));
            // Parted only by moving the small ball wholly out.
            let truth = Truth {
                gap: 0.0,
                depth: 2.0 * r + g,
                faces: g.max(0.0),
            };
            (
                truth,
                format!("ball {r:.6e} in ball {big:.6e}, its centre {e:.6e} off"),
            )
        }
        Kind::BallBox => {
            let size = V::new(
                rng.between(3.0, 12.0) * k,
                rng.between(3.0, 12.0) * k,
                rng.between(3.0, 12.0) * k,
            );
            let r = rng.between(1.0, 4.0) * k;
            if r + g <= 0.05 * k {
                return None;
            }
            let side = |rng: &mut Rng| if rng.below(2) == 0 { -1.0 } else { 1.0 };
            let region = rng.below(3);
            let mut p0 = V::new(
                rng.between(0.1, 0.9) * size.x,
                rng.between(0.1, 0.9) * size.y,
                rng.between(0.1, 0.9) * size.z,
            );
            let mut n = V::new(0.0, 0.0, 0.0);
            let axes: Vec<usize> = match region {
                0 => vec![rng.below(3) as usize],
                1 => {
                    let i = rng.below(3) as usize;
                    vec![i, (i + 1 + rng.below(2) as usize) % 3]
                }
                _ => vec![0, 1, 2],
            };
            for &i in &axes {
                let s = side(rng);
                let w = if axes.len() == 1 {
                    1.0
                } else {
                    rng.between(0.15, 1.0)
                };
                let at = if s > 0.0 { size.get(i) } else { 0.0 };
                p0 = match i {
                    0 => V::new(at, p0.y, p0.z),
                    1 => V::new(p0.x, at, p0.z),
                    _ => V::new(p0.x, p0.y, at),
                };
                n = n + V::axis(i) * (s * w);
            }
            let n = n.unit();
            let c = place(p0, n, r, g);
            steps.push(block("A0", size, V::new(0.0, 0.0, 0.0)));
            steps.push(sphere("B0", r, c));
            // A box wholly inside the ball (its farthest corner within r):
            // the boundaries are r less that apart.
            let far = (0..8)
                .map(|i| {
                    let p = V::new(
                        if i & 1 != 0 { size.x } else { 0.0 },
                        if i & 2 != 0 { size.y } else { 0.0 },
                        if i & 4 != 0 { size.z } else { 0.0 },
                    );
                    (p - c).len()
                })
                .fold(0.0, f64::max);
            let mut truth = Truth::apart(g);
            if far < r {
                truth.faces = r - far;
            }
            let names = ["face", "edge", "corner"];
            (
                truth,
                format!(
                    "ball {r:.6e} at a box's {} ({:.6e} x {:.6e} x {:.6e})",
                    names[region as usize], size.x, size.y, size.z
                ),
            )
        }
        Kind::BallInBox => {
            let size = V::new(
                rng.between(4.0, 12.0) * k,
                rng.between(4.0, 12.0) * k,
                rng.between(4.0, 12.0) * k,
            );
            let c = V::new(
                rng.between(0.3, 0.7) * size.x,
                rng.between(0.3, 0.7) * size.y,
                rng.between(0.3, 0.7) * size.z,
            );
            let depth = box_depth(V::new(0.0, 0.0, 0.0), size, c);
            let r = depth - g;
            if r < 0.3 * k {
                return None;
            }
            steps.push(block("A0", size, V::new(0.0, 0.0, 0.0)));
            steps.push(sphere("B0", r, c));
            let truth = Truth {
                gap: 0.0,
                depth: r,
                faces: g.max(0.0),
            };
            (
                truth,
                format!("ball {r:.6e} inside a box, {depth:.6e} from its nearest wall"),
            )
        }
        Kind::BallCylinder => {
            let (rc, h) = (rng.between(2.0, 6.0) * k, rng.between(4.0, 12.0) * k);
            let r = rng.between(1.0, 4.0) * k;
            if r + g <= 0.05 * k {
                return None;
            }
            let phi = rng.between(0.0, 360.0).to_radians();
            let radial = V::new(phi.pcos(), phi.psin(), 0.0);
            let region = rng.below(3);
            let top = rng.below(2) == 0;
            let (p0, n) = match region {
                0 => (
                    radial * rc + V::new(0.0, 0.0, rng.between(0.2, 0.8) * h),
                    radial,
                ),
                1 => (
                    radial * (rng.between(0.0, 0.8) * rc)
                        + V::new(0.0, 0.0, if top { h } else { 0.0 }),
                    V::new(0.0, 0.0, if top { 1.0 } else { -1.0 }),
                ),
                _ => {
                    let psi = rng.between(5.0, 85.0).to_radians();
                    (
                        radial * rc + V::new(0.0, 0.0, if top { h } else { 0.0 }),
                        radial * psi.pcos()
                            + V::new(0.0, 0.0, if top { psi.psin() } else { -psi.psin() }),
                    )
                }
            };
            let c = place(p0, n, r, g);
            steps.push(cylinder(
                "A0",
                rc,
                h,
                V::new(0.0, 0.0, 0.0),
                V::new(0.0, 0.0, 1.0),
            ));
            steps.push(sphere("B0", r, c));
            let mut truth = Truth::apart(g);
            let rho = c.x.phypot(c.y);
            let far = (rho + rc).phypot(c.z.max(h - c.z));
            if far < r {
                truth.faces = r - far;
            }
            let names = ["wall", "cap", "rim"];
            (
                truth,
                format!(
                    "ball {r:.6e} at a cylinder's {} (radius {rc:.6e}, height {h:.6e})",
                    names[region as usize]
                ),
            )
        }
        Kind::CylParallel => {
            let (r1, r2) = (rng.between(1.0, 5.0) * k, rng.between(1.0, 5.0) * k);
            let h1 = rng.between(5.0, 15.0) * k;
            let (z0, h2) = if rng.below(3) == 0 {
                (0.0, h1)
            } else {
                let z0 = rng.between(-0.3, 0.3) * h1;
                (z0, h1 * rng.between(0.8, 1.3))
            };
            let phi = rng.between(0.0, 360.0).to_radians();
            let d = r1 + r2 + g;
            let axis = V::new(0.0, 0.0, 1.0);
            steps.push(cylinder("A0", r1, h1, V::new(0.0, 0.0, 0.0), axis));
            steps.push(cylinder(
                "B0",
                r2,
                h2,
                V::new(d * phi.pcos(), d * phi.psin(), z0),
                axis,
            ));
            (
                Truth::apart(g),
                format!(
                    "parallel cylinders {r1:.6e} and {r2:.6e}, heights {h1:.6e} and {h2:.6e} from {z0:.6e}"
                ),
            )
        }
        Kind::CylSkew => {
            let (r1, r2) = (rng.between(1.0, 4.0) * k, rng.between(1.0, 4.0) * k);
            let theta = if rng.below(3) == 0 {
                90.0
            } else {
                rng.between(15.0, 89.0)
            };
            let t = theta.to_radians();
            let len = 2.0 * 3.0 * (r1 + r2 + 1.0 * k) / t.psin() + 4.0 * k;
            let d = r1 + r2 + g;
            let w = V::new(0.0, t.psin(), t.pcos());
            steps.push(cylinder(
                "A0",
                r1,
                len,
                V::new(0.0, 0.0, -0.5 * len),
                V::new(0.0, 0.0, 1.0),
            ));
            steps.push(cylinder(
                "B0",
                r2,
                len,
                V::new(d, 0.0, 0.0) - w * (0.5 * len),
                w,
            ));
            (
                Truth::apart(g),
                format!("cylinders {r1:.6e} and {r2:.6e} crossing at {theta:.3} deg"),
            )
        }
        Kind::PinHole => {
            let w = rng.between(8.0, 16.0) * k;
            let t = rng.between(3.0, 10.0) * k;
            let rh = rng.between(1.5, 0.3 * w / k) * k;
            let e = if rng.below(3) == 0 {
                0.0
            } else {
                rng.between(0.0, 0.5) * rh
            };
            let rp = rh - e - g;
            if rp < 0.3 * k || rp > 0.45 * w {
                return None;
            }
            let phi = rng.between(0.0, 360.0).to_radians();
            let axis = V::new(0.0, 0.0, 1.0);
            let mid = V::new(0.5 * w, 0.5 * w, 0.0);
            steps.push(block("Blank", V::new(w, w, t), V::new(0.0, 0.0, 0.0)));
            steps.push(cylinder(
                "Drill",
                rh,
                3.0 * t,
                mid - V::new(0.0, 0.0, t),
                axis,
            ));
            steps.push(subtract("A0", "Blank", "Drill"));
            steps.push(cylinder(
                "B0",
                rp,
                3.0 * t,
                mid + V::new(e * phi.pcos(), e * phi.psin(), -t),
                axis,
            ));
            (
                Truth::apart(g),
                format!("pin {rp:.6e} in a hole {rh:.6e}, {e:.6e} off its axis"),
            )
        }
        Kind::TorusPlane => {
            let big = rng.between(3.0, 8.0) * k;
            let small = rng.between(0.5, 0.6 * big / k) * k;
            let theta = match rng.below(8) {
                0 | 1 => 0.0,
                2 => 90.0,
                _ => rng.between(0.0, 80.0),
            };
            let psi = rng.between(0.0, 360.0).to_radians();
            let t = theta.to_radians();
            let axis = V::new(t.psin() * psi.pcos(), t.psin() * psi.psin(), t.pcos());
            let (cx, cy) = (rng.between(-2.0, 2.0) * k, rng.between(-2.0, 2.0) * k);
            let hc = big * t.psin() + small + g;
            let l = 2.0 * (big + small + 4.0 * k);
            let depth = rng.between(2.0, 5.0) * k;
            steps.push(block(
                "A0",
                V::new(l, l, depth),
                V::new(-0.5 * l, -0.5 * l, -depth),
            ));
            steps.push((
                "feature.torus",
                json!({"name": "B0", "major": mm(big), "minor": mm(small), "position": mm3(V::new(cx, cy, hc)), "axis": num3(axis)}),
            ));
            (
                Truth::apart(g),
                format!("ring {big:.6e}, {small:.6e} tilted {theta:.3} deg over a plane"),
            )
        }
        Kind::TorusBall => {
            let big = rng.between(3.0, 8.0) * k;
            let small = rng.between(0.5, 0.6 * big / k) * k;
            let on_axis = rng.below(3) == 0;
            let rho = if on_axis {
                0.0
            } else {
                rng.between(0.0, 2.5) * big
            };
            let z0 = rng.between(-1.0, 1.0) * big;
            let core = if on_axis {
                big.phypot(z0)
            } else {
                (rho - big).phypot(z0)
            };
            if core < small + 0.3 * k {
                return None;
            }
            let rb = core - small - g;
            if rb < 0.3 * k {
                return None;
            }
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push((
                "feature.torus",
                json!({"name": "A0", "major": mm(big), "minor": mm(small)}),
            ));
            steps.push(sphere(
                "B0",
                rb,
                V::new(rho * phi.pcos(), rho * phi.psin(), z0),
            ));
            (
                Truth::apart(g),
                format!(
                    "ball {rb:.6e} at ({rho:.6e} off the axis, {z0:.6e} up) by a ring {big:.6e}, {small:.6e}"
                ),
            )
        }
        Kind::ConePlane => {
            let rc = rng.between(2.0, 6.0) * k;
            let h = rng.between(3.0, 10.0) * k;
            let theta = match rng.below(8) {
                0 | 1 => 0.0,
                2 | 3 => 180.0,
                _ => rng.between(0.0, 170.0),
            };
            let psi = rng.between(0.0, 360.0).to_radians();
            let t = theta.to_radians();
            let axis = V::new(t.psin() * psi.pcos(), t.psin() * psi.psin(), t.pcos());
            // The lowest point: the apex, or the base circle's lowest.
            let rim = (1.0 - axis.z * axis.z).max(0.0).sqrt();
            let low = (-rc * rim).min(h * axis.z);
            let base = V::new(
                rng.between(-2.0, 2.0) * k,
                rng.between(-2.0, 2.0) * k,
                g - low,
            );
            let l = 2.0 * (rc + h + 4.0 * k);
            let depth = rng.between(2.0, 5.0) * k;
            steps.push(block(
                "A0",
                V::new(l, l, depth),
                V::new(-0.5 * l, -0.5 * l, -depth),
            ));
            steps.push((
                "feature.cone",
                json!({"name": "B0", "radius": mm(rc), "height": mm(h), "position": mm3(base), "axis": num3(axis)}),
            ));
            (
                Truth::apart(g),
                format!("cone {rc:.6e}, height {h:.6e}, tilted {theta:.3} deg over a plane"),
            )
        }
        Kind::BoxBox | Kind::ThinPlates => {
            let thin = kind == Kind::ThinPlates;
            let dims = |rng: &mut Rng, lo: f64, hi: f64| {
                let mut d = [
                    rng.between(lo, hi) * k,
                    rng.between(lo, hi) * k,
                    rng.between(lo, hi) * k,
                ];
                if thin {
                    d[2] = rng.between(0.05, 0.5) * k;
                }
                d
            };
            let a = dims(rng, 3.0, 12.0);
            let b = dims(rng, 2.0, 10.0);
            let turn = match rng.below(4) {
                0 => Turn::NONE,
                1 => Turn {
                    axis: V::axis(rng.below(3) as usize),
                    deg: rng.between(-180.0, 180.0),
                },
                2 if thin => Turn {
                    axis: V::new(1.0, 0.0, 0.0),
                    deg: 90.0,
                },
                _ => Turn::random(rng),
            };
            let oa = Obb::axis_aligned(V::new(0.0, 0.0, 0.0), V::new(a[0], a[1], a[2]));
            let ob_at = |s: f64, u: V| Obb {
                c: oa.c + u * s,
                u: [
                    turn.apply(V::axis(0)),
                    turn.apply(V::axis(1)),
                    turn.apply(V::axis(2)),
                ],
                h: [0.5 * b[0], 0.5 * b[1], 0.5 * b[2]],
            };
            let u = rng.direction();
            // Where B, moved from A's middle along u, stops sharing volume.
            let (mut lo, mut hi) = (0.0, 2.0 * (a[0] + a[1] + a[2] + b[0] + b[1] + b[2]));
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if obb_overlap(&oa, &ob_at(mid, u), 0.0) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let s = if g >= 0.0 {
                // The distance grows convexly from 0 at hi: find g.
                let (mut l2, mut h2) = (hi, hi + g + 1e-9 * k);
                while obb_distance(&oa, &ob_at(h2, u)) < g {
                    h2 += g + k;
                }
                for _ in 0..200 {
                    let mid = 0.5 * (l2 + h2);
                    if obb_distance(&oa, &ob_at(mid, u)) < g {
                        l2 = mid;
                    } else {
                        h2 = mid;
                    }
                }
                h2
            } else if thin && rng.below(3) == 0 {
                // Through each other at their middles.
                0.0
            } else {
                (hi + g).max(0.0)
            };
            let ob = ob_at(s, u);
            let overlap = obb_overlap(&oa, &ob, 0.0);
            let truth = if overlap {
                // The least of the separating overlaps: the depth.
                let mut axes: Vec<V> = Vec::new();
                axes.extend(oa.u);
                axes.extend(ob.u);
                for i in 0..3 {
                    for j in 0..3 {
                        let c = oa.u[i].cross(ob.u[j]);
                        if c.len() > 1e-9 {
                            axes.push(c.unit());
                        }
                    }
                }
                let t = ob.c - oa.c;
                let depth = axes
                    .iter()
                    .map(|&ax| {
                        let ra: f64 = (0..3).map(|i| oa.h[i] * oa.u[i].dot(ax).abs()).sum();
                        let rb: f64 = (0..3).map(|i| ob.h[i] * ob.u[i].dot(ax).abs()).sum();
                        ra + rb - t.dot(ax).abs()
                    })
                    .fold(f64::INFINITY, f64::min);
                Truth {
                    gap: 0.0,
                    depth,
                    faces: obb_boundary_distance(&oa, &ob),
                }
            } else {
                Truth::apart(obb_distance(&oa, &ob))
            };
            steps.push(block("A0", V::new(a[0], a[1], a[2]), V::new(0.0, 0.0, 0.0)));
            steps.push(block(
                "B1",
                V::new(b[0], b[1], b[2]),
                V::new(-0.5 * b[0], -0.5 * b[1], -0.5 * b[2]),
            ));
            steps.push((
                "feature.transform",
                json!({"source": "B1", "name": "B0", "angle": format!("{:?} deg", turn.deg), "axis": num3(turn.axis), "translate": mm3(ob.c)}),
            ));
            (
                truth,
                format!(
                    "{} {:.4e} x {:.4e} x {:.4e} and {:.4e} x {:.4e} x {:.4e} turned {:.3} deg",
                    if thin { "plates" } else { "boxes" },
                    a[0],
                    a[1],
                    a[2],
                    b[0],
                    b[1],
                    b[2],
                    turn.deg
                ),
            )
        }
        Kind::BallInHollow => {
            let size = V::new(
                rng.between(10.0, 20.0) * k,
                rng.between(10.0, 20.0) * k,
                rng.between(10.0, 20.0) * k,
            );
            let mid = size * 0.5;
            let spherical = rng.below(2) == 0;
            let wall = 0.35 * size.x.min(size.y).min(size.z);
            steps.push(block("Blank", size, V::new(0.0, 0.0, 0.0)));
            // The hollow's centre, the ball's centre, and how far the
            // ball's sphere is from the hollow's boundary given its radius
            // (0 where they cross).
            let lo_b = V::new(0.0, 0.0, 0.0);
            let (c, room, cavity, hollow_faces): (V, f64, String, Box<dyn Fn(f64) -> f64>) =
                if spherical {
                    let rc = rng.between(2.0, wall / k) * k;
                    let e = rng.between(0.0, 0.5) * rc;
                    steps.push(sphere("Cavity", rc, mid));
                    let c = mid + rng.direction() * e;
                    let e = (c - mid).len();
                    (
                        c,
                        rc - e,
                        format!("a round hollow {rc:.6e}, the ball {e:.6e} off its middle"),
                        // Two spheres e apart: apart, one holding the
                        // other, or crossing.
                        Box::new(move |rb: f64| {
                            if e >= rb + rc {
                                e - rb - rc
                            } else if e <= (rb - rc).abs() {
                                (rb - rc).abs() - e
                            } else {
                                0.0
                            }
                        }),
                    )
                } else {
                    let cs = V::new(
                        rng.between(3.0, 2.0 * wall / k) * k,
                        rng.between(3.0, 2.0 * wall / k) * k,
                        rng.between(3.0, 2.0 * wall / k) * k,
                    );
                    let lo = mid - cs * 0.5;
                    steps.push(block("Cavity", cs, lo));
                    let c = lo
                        + V::new(
                            rng.between(0.3, 0.7) * cs.x,
                            rng.between(0.3, 0.7) * cs.y,
                            rng.between(0.3, 0.7) * cs.z,
                        );
                    let depth = box_depth(lo, lo + cs, c);
                    let far = Obb::axis_aligned(lo, lo + cs)
                        .corners()
                        .iter()
                        .map(|&p| (p - c).len())
                        .fold(0.0, f64::max);
                    (
                        c,
                        depth,
                        format!("a box hollow {:.4e} x {:.4e} x {:.4e}", cs.x, cs.y, cs.z),
                        // The centre inside the box: the sphere inside it,
                        // holding it, or crossing its faces.
                        Box::new(move |rb: f64| {
                            if rb <= depth {
                                depth - rb
                            } else if rb >= far {
                                rb - far
                            } else {
                                0.0
                            }
                        }),
                    )
                };
            let rb = room - g;
            if rb < 0.3 * k {
                return None;
            }
            steps.push(subtract("A0", "Blank", "Cavity"));
            steps.push(sphere("B0", rb, c));
            // The outer box's faces too: the ball inside the block, or
            // through its outside.
            let outer = box_depth(lo_b, size, c) - rb;
            let truth = Truth {
                faces: hollow_faces(rb).min(outer.max(0.0)),
                ..Truth::apart(g)
            };
            (truth, format!("ball {rb:.6e} in {cavity}"))
        }
        Kind::BallCone => {
            let rc = rng.between(1.0, 5.0) * k;
            let h = rng.between(1.0, 10.0) * k;
            // The meridian half-section: a triangle (0, 0), (rc, 0), (0, h).
            let tri = [(0.0, 0.0), (rc, 0.0), (0.0, h)];
            let seg = |p: (f64, f64), a: (f64, f64), b: (f64, f64)| {
                let (pa, ab) = ((p.0 - a.0, p.1 - a.1), (b.0 - a.0, b.1 - a.1));
                let t = ((pa.0 * ab.0 + pa.1 * ab.1) / (ab.0 * ab.0 + ab.1 * ab.1)).clamp(0.0, 1.0);
                (pa.0 - t * ab.0).phypot(pa.1 - t * ab.1)
            };
            // A point outside it: from the apex, the rim or the side's
            // middle, out along a direction in that place's normal cone.
            let sl = h.phypot(rc);
            let side_n = (h / sl, rc / sl);
            let a0 = side_n.1.patan2(side_n.0);
            let (from, dir) = match rng.below(3) {
                0 => {
                    // Apex: between the side's normal and straight up.
                    let a = rng.between(a0, PI / 2.0);
                    ((0.0, h), (a.pcos(), a.psin()))
                }
                1 => {
                    // Rim: between straight down and the side's normal.
                    let a = rng.between(-PI / 2.0, a0);
                    ((rc, 0.0), (a.pcos(), a.psin()))
                }
                _ => {
                    let t = rng.between(0.1, 0.9);
                    ((rc * (1.0 - t), h * t), side_n)
                }
            };
            let d = rng.between(0.5, 4.0) * k;
            let p = (from.0 + dir.0 * d, from.1 + dir.1 * d);
            if p.0 < 0.0 {
                return None;
            }
            let near = seg(p, tri[0], tri[1])
                .min(seg(p, tri[1], tri[2]))
                .min(seg(p, tri[2], tri[0]));
            let rb = near - g;
            if rb < 0.3 * k {
                return None;
            }
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push((
                "feature.cone",
                json!({"name": "A0", "radius": mm(rc), "height": mm(h)}),
            ));
            steps.push(sphere(
                "B0",
                rb,
                V::new(p.0 * phi.pcos(), p.0 * phi.psin(), p.1),
            ));
            (
                Truth::apart(g),
                format!(
                    "ball {rb:.6e} at ({:.4e}, {:.4e}) by a cone {rc:.6e}, {h:.6e}",
                    p.0, p.1
                ),
            )
        }
        Kind::TorusPin => {
            let big = rng.between(3.0, 8.0) * k;
            let small = rng.between(0.3, 0.5) * big;
            let e = if rng.below(3) == 0 {
                0.0
            } else {
                rng.between(0.0, 0.4) * (big - small)
            };
            let rp = big - e - small - g;
            if rp < 0.2 * k {
                return None;
            }
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push((
                "feature.torus",
                json!({"name": "A0", "major": mm(big), "minor": mm(small)}),
            ));
            steps.push(cylinder(
                "B0",
                rp,
                6.0 * small,
                V::new(e * phi.pcos(), e * phi.psin(), -3.0 * small),
                V::new(0.0, 0.0, 1.0),
            ));
            (
                Truth::apart(g),
                format!("pin {rp:.6e} through a ring {big:.6e}, {small:.6e}, {e:.6e} off its axis"),
            )
        }
        Kind::BigBall | Kind::Mismatch => {
            let big = kind == Kind::BigBall;
            let (size, rb) = if big {
                (
                    V::new(20.0 * k, 20.0 * k, 5.0 * k),
                    rng.pick(&[50.0, 500.0, 5000.0]) * k * rng.between(1.0, 2.0),
                )
            } else {
                let l = rng.pick(&[100.0, 1000.0]) * k;
                (
                    V::new(l, l * rng.between(0.5, 1.0), 0.1 * l),
                    rng.between(0.01, 0.5) * k,
                )
            };
            if rb + g <= 0.05 * rb {
                return None;
            }
            let at = V::new(
                rng.between(0.4, 0.6) * size.x,
                rng.between(0.4, 0.6) * size.y,
                size.z + rb + g,
            );
            steps.push(block("A0", size, V::new(0.0, 0.0, 0.0)));
            steps.push(sphere("B0", rb, at));
            (
                Truth::apart(g),
                format!(
                    "ball {rb:.6e} over a block {:.4e} x {:.4e} x {:.4e}",
                    size.x, size.y, size.z
                ),
            )
        }
        Kind::Vase => {
            analytic = false;
            let l = rng.between(4.0, 16.0) * k;
            let d = rng.between(0.3, 0.6) * l;
            let u0 = d + rng.between(0.2, 1.0) * l;
            let (h1, h2) = (rng.between(-0.3, 0.8) * d, rng.between(-0.3, 0.8) * d);
            let curve = Bezier([
                [u0, 0.0],
                [u0 + h1, l / 3.0],
                [u0 + h2, 2.0 * l / 3.0],
                [u0, l],
            ]);
            // A point outside the profile, off the bulging wall, in the
            // meridian plane (the curve runs up, so outward is to its
            // right).
            let t0 = rng.between(0.1, 0.9);
            let p = curve.at(t0);
            let dd = curve.d(t0);
            let n = [dd[1], -dd[0]];
            let nl = n[0].phypot(n[1]);
            let rb = rng.between(0.5, 3.0) * k;
            if rb + g <= 0.05 * k {
                return None;
            }
            let c = [p[0] + n[0] / nl * (rb + g), p[1] + n[1] / nl * (rb + g)];
            // Inside: left of the curve at its height (a graph over the
            // height: its control points are evenly spaced along it) and
            // right of the inner wall.
            let inside =
                (0.0..=l).contains(&c[1]) && c[0] <= curve.at(c[1] / l)[0] && c[0] >= u0 - d;
            if inside {
                return None;
            }
            let (near, _) = curve.nearest(c);
            let q = V::new(c[0], c[1], 0.0);
            let lines = [
                ([u0, l], [u0 - d, l]),
                ([u0 - d, l], [u0 - d, 0.0]),
                ([u0 - d, 0.0], [u0, 0.0]),
            ]
            .iter()
            .map(|(a, b)| {
                (q - segment_nearest(V::new(a[0], a[1], 0.0), V::new(b[0], b[1], 0.0), q)).len()
            })
            .fold(f64::INFINITY, f64::min);
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push(("sketch.create", json!({"name": "PV", "plane": "xy"})));
            steps.push((
                "sketch.polyline",
                json!({"sketch": "PV", "points": [[u0, l], [u0 - d, l], [u0 - d, 0.0], [u0, 0.0]]}),
            ));
            steps.push((
                "sketch.spline",
                json!({"sketch": "PV", "control": ["$last", [u0 + h1, l / 3.0], [u0 + h2, 2.0 * l / 3.0], "$first"]}),
            ));
            steps.push((
                "feature.revolve",
                json!({"sketch": "PV", "axis": "y", "name": "A0"}),
            ));
            // The sketch's x is the radius, its y the axis: the ball's
            // centre at radius c[0], height c[1], turned φ about y.
            steps.push(sphere(
                "B0",
                rb,
                V::new(c[0] * phi.pcos(), c[1], c[0] * phi.psin()),
            ));
            (
                Truth::apart(near.min(lines) - rb),
                format!(
                    "ball {rb:.6e} by a vase (profile {l:.4e} long, {u0:.4e} out, bulges {h1:.4e}, {h2:.4e}) at t {t0:.3}"
                ),
            )
        }
        Kind::Lumps => {
            // A block with a round hollow at its middle, and a body of
            // two balls (a pattern of one): each ball outside the block,
            // in the hollow, or wholly in the material.
            let side = rng.between(10.0, 20.0) * k;
            let size = V::new(side, side, side);
            let m = size * 0.5;
            let rc = rng.between(0.2, 0.3) * side;
            let r = rng.between(0.3, 0.6) * k;
            steps.push(block("Blank", size, V::new(0.0, 0.0, 0.0)));
            steps.push(sphere("Cavity", rc, m));
            steps.push(subtract("A0", "Blank", "Cavity"));
            // Where a ball goes, and what it is to the block: its centre,
            // its signed gap (negative: how deep it overlaps), its
            // sphere's distance from the block's faces.
            let place_one = |rng: &mut Rng, g: f64| -> (V, f64, f64, &'static str) {
                match rng.below(3) {
                    0 => {
                        // Outside, over the top face, g from it.
                        let c = V::new(
                            rng.between(0.2, 0.8) * side,
                            rng.between(0.2, 0.8) * side,
                            side + r + g,
                        );
                        (c, g, g.max(0.0), "outside")
                    }
                    1 => {
                        // In the hollow, g from its wall.
                        let e = (rc - r - g).max(0.0);
                        let c = m + rng.direction() * e;
                        let gap = rc - (c - m).len() - r;
                        (c, gap, gap.max(0.0), "in the hollow")
                    }
                    _ => {
                        // Wholly in the material, between the hollow and
                        // the outside, clear of both.
                        let dir = rng.direction();
                        let from = rc + r + 0.25 * (0.5 * side - rc - 2.0 * r);
                        let c = m + dir * from;
                        let faces = ((c - m).len() - rc - r)
                            .min(box_depth(V::new(0.0, 0.0, 0.0), size, c) - r);
                        (c, -r, faces.max(0.0), "in the material")
                    }
                }
            };
            let (c1, s1, f1, w1) = place_one(rng, g);
            let g2 = rng.pick(&GAPS) * k;
            let (c2, s2, f2, w2) = place_one(rng, g2);
            if (c1 - c2).len() < 2.0 * r + 0.1 * k {
                return None;
            }
            steps.push(sphere("B1", r, c1));
            steps.push((
                "feature.pattern",
                json!({"source": "B1", "count": 2, "step": mm3(c2 - c1), "name": "B0"}),
            ));
            let truth = Truth {
                faces: f1.min(f2),
                ..Truth::apart(s1.min(s2))
            };
            (
                truth,
                format!(
                    "two balls {r:.6e}, {w1} ({s1:.3e}) and {w2} ({s2:.3e}), by a block {side:.4e} with a hollow {rc:.6e}"
                ),
            )
        }
        Kind::ConeBigBall => {
            // A slender or ordinary cone and a ball 10 to 500 times the
            // scale against its side, off a point of the side along its
            // normal (the nearest point in the meridian plane: the side
            // is straight there).
            let rc = rng.between(0.2, 3.0) * k;
            let h = rng.between(5.0, 20.0) * k;
            let rb = rng.pick(&[10.0, 50.0, 200.0, 500.0]) * k * rng.between(1.0, 2.0);
            if rb + g <= 0.0 {
                return None;
            }
            let t = rng.between(0.1, 0.9);
            let sl = h.phypot(rc);
            let (foot, n) = ((rc * (1.0 - t), h * t), (h / sl, rc / sl));
            let p = (foot.0 + n.0 * (rb + g), foot.1 + n.1 * (rb + g));
            let phi = rng.between(0.0, 360.0).to_radians();
            steps.push((
                "feature.cone",
                json!({"name": "A0", "radius": mm(rc), "height": mm(h)}),
            ));
            steps.push(sphere(
                "B0",
                rb,
                V::new(p.0 * phi.pcos(), p.0 * phi.psin(), p.1),
            ));
            (
                Truth::apart(g),
                format!("ball {rb:.6e} by a cone {rc:.6e}, {h:.6e} at {t:.3} of its height"),
            )
        }
        Kind::SplineBall | Kind::SplineSpline => {
            analytic = false;
            let l = rng.between(8.0, 16.0) * k;
            let (h1, h2) = (rng.between(-0.3, 0.6) * l, rng.between(-0.3, 0.6) * l);
            let depth = 0.6 * l;
            let height = rng.between(4.0, 12.0) * k;
            let a = Bezier([[0.0, 0.0], [l / 3.0, h1], [2.0 * l / 3.0, h2], [l, 0.0]]);
            let mut boundary_a = vec![
                ([l, 0.0], [l, -depth]),
                ([l, -depth], [0.0, -depth]),
                ([0.0, -depth], [0.0, 0.0]),
            ];
            steps.extend(spline_prism("A0", "PA", &a, depth, height, 0.0));
            if kind == Kind::SplineBall {
                let r = rng.between(0.5, 3.0) * k;
                if r + g <= 0.05 * k {
                    return None;
                }
                let t0 = rng.between(0.05, 0.95);
                let p = a.at(t0);
                let d = a.d(t0);
                let n = [-d[1], d[0]];
                let nl = n[0].phypot(n[1]);
                let c = [p[0] + n[0] / nl * (r + g), p[1] + n[1] / nl * (r + g)];
                // Outside the region below the curve, or skipped.
                if (0.0..=l).contains(&c[0]) && c[1] <= a.at(c[0] / l)[1] {
                    return None;
                }
                let (near, _) = a.nearest(c);
                let lines = boundary_a
                    .drain(..)
                    .map(|(p, q)| {
                        (V::new(c[0], c[1], 0.0)
                            - segment_nearest(
                                V::new(p[0], p[1], 0.0),
                                V::new(q[0], q[1], 0.0),
                                V::new(c[0], c[1], 0.0),
                            ))
                        .len()
                    })
                    .fold(f64::INFINITY, f64::min);
                let d2 = near.min(lines);
                let z = rng.between(0.3, 0.7) * height;
                steps.push(sphere("B0", r, V::new(c[0], c[1], z)));
                (
                    Truth::apart(d2 - r),
                    format!(
                        "ball {r:.6e} at a spline wall (length {l:.6e}, control heights {h1:.6e}, {h2:.6e}) at t {t0:.4}"
                    ),
                )
            } else {
                // A second prism above: its region above a curve that
                // bends down, its ends at y0.
                let l2 = rng.between(0.6, 1.2) * l;
                let dx = rng.between(-0.2, 0.4) * l;
                let (q1, q2) = (rng.between(-0.6, 0.3) * l2, rng.between(-0.6, 0.3) * l2);
                let bez = |y0: f64| {
                    Bezier([
                        [dx, y0],
                        [dx + l2 / 3.0, y0 + q1],
                        [dx + 2.0 * l2 / 3.0, y0 + q2],
                        [dx + l2, y0],
                    ])
                };
                let truth_at = |y0: f64| -> (f64, f64, f64) {
                    // Vertical overlap of the regions (both curves are
                    // graphs over x: their control points are evenly
                    // spaced along it), its least refined; and the
                    // distance of the curves where they are apart.
                    let b = bez(y0);
                    let (gap, x) =
                        least_of(dx, dx + l2, |x| b.at((x - dx) / l2)[1] - a.at(x / l)[1]);
                    let over = -gap;
                    // The steeper slope there: two curves crossing by a
                    // vertical `over` are parted by no less than `over`
                    // times its cosine.
                    let (da, db) = (a.d(x / l), b.d((x - dx) / l2));
                    let slope = (da[1] / da[0]).abs().max((db[1] / db[0]).abs());
                    // The curves, and each curve against the other prism's
                    // straight sides (the lower one's go down from its
                    // ends, the upper one's up).
                    let sides_a = [([0.0, 0.0], [0.0, -depth]), ([l, 0.0], [l, -depth])];
                    let sides_b = [
                        ([dx, y0], [dx, y0 + depth]),
                        ([dx + l2, y0], [dx + l2, y0 + depth]),
                    ];
                    let mut d = bezier_bezier(&a, &b);
                    for (p, q) in sides_a {
                        d = d.min(bezier_segment(&b, p, q));
                    }
                    for (p, q) in sides_b {
                        d = d.min(bezier_segment(&a, p, q));
                    }
                    (over, d, slope)
                };
                // y0 so that the curves are about g apart (or overlap by
                // about -g), by a few secant steps on the vertical gap.
                let mut y0 = 0.6 * l;
                for _ in 0..4 {
                    let (over, dist, _) = truth_at(y0);
                    let now = if over > 0.0 { -over } else { dist };
                    y0 += g - now;
                }
                let (over, dist, slope) = truth_at(y0);
                let b = bez(y0);
                // The curves alone must be what is nearest: the ends of
                // each above or below the other's region, and the straight
                // sides far from the other; otherwise drawn again.
                if dx < 0.0 || dx + l2 > l {
                    // An end of the upper curve beyond the lower one: its
                    // side then faces the lower prism's corner; leave out.
                    return None;
                }
                steps.extend(spline_prism("B0", "PB", &b, -depth, height, 0.0));
                let truth = if over > 0.0 {
                    Truth {
                        gap: 0.0,
                        depth: over / (1.0 + slope * slope).sqrt(),
                        faces: 0.0,
                    }
                } else {
                    Truth::apart(dist)
                };
                (
                    truth,
                    format!(
                        "spline walls (lengths {l:.6e}, {l2:.6e}; heights {h1:.4e}, {h2:.4e} and {q1:.4e}, {q2:.4e}; offset {dx:.4e}, {y0:.6e})"
                    ),
                )
            }
        }
    };
    moved(&mut steps, &motion);
    Some(Case {
        id: format!("{kind:?} {index}"),
        what: format!("{what}, gap {g:.3e} at scale {k:e}, {}", motion.describe()),
        k,
        steps,
        truth,
        motion,
        analytic,
    })
}

/// A prism on the profile of the curve `b` (from its first point to its
/// last) closed by three lines `depth` below (or above, for a negative
/// depth) its ends, extruded `height` up from `z0` along z.
fn spline_prism(
    name: &str,
    sketch: &str,
    b: &Bezier,
    depth: f64,
    height: f64,
    z0: f64,
) -> Vec<Step> {
    let p = b.0;
    let (s, e) = (p[0], p[3]);
    vec![
        (
            "sketch.create",
            json!({"name": sketch, "plane": "xy", "offset": mm(z0)}),
        ),
        (
            "sketch.polyline",
            json!({"sketch": sketch, "points": [[e[0], e[1]], [e[0], e[1] - depth], [s[0], s[1] - depth], [s[0], s[1]]]}),
        ),
        (
            "sketch.spline",
            json!({"sketch": sketch, "control": ["$last", [p[1][0], p[1][1]], [p[2][0], p[2][1]], "$first"]}),
        ),
        (
            "feature.extrude",
            json!({"sketch": sketch, "distance": mm(height), "name": name}),
        ),
    ]
}

/// Runs the steps of a case, putting the polyline's last and first points
/// in for `$last` and `$first` (the spline closes the profile on them).
fn build_case(s: &mut craft3d_engine::Session, steps: &[Step]) -> Result<(), Ran> {
    let mut points: Vec<Value> = Vec::new();
    for (id, params) in steps {
        let mut params = params.clone();
        if *id == "sketch.spline" {
            let control = params["control"].as_array_mut().unwrap();
            for c in control.iter_mut() {
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

/// What the clearance said, one way.
#[derive(Clone, Debug)]
struct Said {
    status: String,
    value: Option<f64>,
    at_least: Option<f64>,
}

fn clearance(s: &mut craft3d_engine::Session, a: &str, b: &str) -> Result<Said, Ran> {
    match run(s, "measure.clearance", &json!({"from": a, "to": b})) {
        Ran::Ok(out) => Ok(Said {
            status: out["pairs"][0]["status"]
                .as_str()
                .unwrap_or("?")
                .to_string(),
            value: out["clearance"].as_f64(),
            at_least: out["atLeast"].as_f64(),
        }),
        other => Err(other),
    }
}

fn distance(s: &mut craft3d_engine::Session, a: &str, b: &str) -> Result<(f64, f64), Ran> {
    match run(s, "measure.distance", &json!({"from": a, "to": b})) {
        Ran::Ok(out) => match (out["distance"].as_f64(), out["atLeast"].as_f64()) {
            (Some(d), Some(l)) => Ok((d, l)),
            _ => Err(Ran::Err(format!("no distance in {out}"))),
        },
        other => Err(other),
    }
}

/// The outcome of a case and what was wrong, or `None` when the engine
/// did not build it.
fn judge(case: &Case) -> Option<(Verdict, String)> {
    let mut s = document(case.k);
    match build_case(&mut s, &case.steps) {
        Ok(()) => {}
        Err(Ran::Panic(p)) => return Some((Verdict::Panic, format!("building: {p}"))),
        Err(_) => return None,
    }
    let tol = 1e-6 * case.k;
    let t = case.truth;
    // The rounding of the bodies' own coordinates, far from the origin.
    let eps = 1e-13 * (case.motion.reach() + 50.0 * case.k) + 1e-12 * t.faces;
    let mut wrong: Vec<String> = Vec::new();
    let mut declined: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut panicked: Vec<String> = Vec::new();
    let mut statuses = Vec::new();
    for (a, b) in [("A", "B"), ("B", "A")] {
        match clearance(&mut s, a, b) {
            Err(Ran::Panic(p)) => panicked.push(format!("clearance {a} {b}: {p}")),
            Err(Ran::Err(e)) => declined.push(format!("clearance {a} {b}: {e}")),
            Err(Ran::Ok(v)) => declined.push(format!("distance: {v}")),
            Ok(said) => {
                statuses.push(said.status.clone());
                let tag = format!("clearance {a} {b}");
                match said.status.as_str() {
                    "overlap" => {
                        if t.depth == 0.0 && t.gap > tol {
                            wrong
                                .push(format!("{tag}: overlap said of bodies {:.6e} apart", t.gap));
                        }
                        if said.value.is_some() || said.at_least.is_some() {
                            wrong.push(format!("{tag}: overlap with a value {said:?}"));
                        }
                    }
                    "touch" => {
                        if t.depth > 2.0 * tol {
                            wrong.push(format!(
                                "{tag}: touch said of bodies overlapping {:.6e} deep",
                                t.depth
                            ));
                        }
                        if t.depth == 0.0 && t.gap > 4.0 * tol {
                            wrong.push(format!("{tag}: touch said of bodies {:.6e} apart", t.gap));
                        }
                    }
                    "clear" => {
                        if t.depth > tol {
                            wrong.push(format!(
                                "{tag}: clear ({said:?}) said of bodies sharing volume {:.6e} deep",
                                t.depth
                            ));
                        }
                        match (said.value, said.at_least) {
                            (Some(v), Some(l)) => {
                                if l > t.gap + eps {
                                    wrong.push(format!(
                                        "{tag}: atLeast {l:.17e} above the true distance {:.17e} (by {:.3e})",
                                        t.gap,
                                        l - t.gap
                                    ));
                                }
                                if l > v {
                                    wrong.push(format!(
                                        "{tag}: atLeast {l:e} above the value {v:e}"
                                    ));
                                }
                                if v < t.gap - 2.0 * tol - eps {
                                    wrong.push(format!(
                                        "{tag}: clearance {v:.17e} below the true distance {:.17e}",
                                        t.gap
                                    ));
                                }
                                let acc = 2.0 * tol + 1e-9 * t.gap + eps;
                                if v > t.gap + acc {
                                    if case.analytic && v <= t.gap + acc + 8.0 * tol {
                                        notes.push(format!(
                                            "inexact: {tag}: clearance {v:.17e} above the true distance {:.17e} on analytic faces by {:.3e} ({:.1} tolerances)",
                                            t.gap,
                                            v - t.gap,
                                            (v - t.gap) / tol
                                        ));
                                    } else if case.analytic {
                                        wrong.push(format!(
                                            "{tag}: clearance {v:.17e} above the true distance {:.17e} on analytic faces (by {:.3e})",
                                            t.gap,
                                            v - t.gap
                                        ));
                                    } else {
                                        notes.push(format!(
                                            "{tag}: clearance {v:.6e} short of the true {:.6e} on a spline (documented)",
                                            t.gap
                                        ));
                                    }
                                }
                            }
                            _ => wrong.push(format!("{tag}: clear without a value {said:?}")),
                        }
                    }
                    other => declined.push(format!("{tag}: status {other}")),
                }
            }
        }
    }
    if statuses.len() == 2 && statuses[0] != statuses[1] {
        let band = t.gap <= 4.0 * tol && t.depth <= 2.0 * tol;
        let line = format!(
            "clearance depends on the order: {} one way, {} the other",
            statuses[0], statuses[1]
        );
        if band {
            notes.push(line);
        } else {
            wrong.push(line);
        }
    }
    match distance(&mut s, "A", "B") {
        Err(Ran::Panic(p)) => panicked.push(format!("distance: {p}")),
        Err(Ran::Err(e)) => declined.push(format!("distance: {e}")),
        Err(Ran::Ok(v)) => declined.push(format!("distance: {v}")),
        Ok((d, l)) => {
            if l > t.faces + eps {
                wrong.push(format!(
                    "distance: atLeast {l:.17e} above the true distance of the faces {:.17e} (by {:.3e})",
                    t.faces,
                    l - t.faces
                ));
            }
            if d < t.faces - 2.0 * tol - eps {
                wrong.push(format!(
                    "distance: {d:.17e} below the true distance of the faces {:.17e}",
                    t.faces
                ));
            }
            let acc = 2.0 * tol + 1e-9 * t.faces + eps;
            if d > t.faces + acc {
                if case.analytic && d <= t.faces + acc + 8.0 * tol {
                    notes.push(format!(
                        "inexact: distance: {d:.17e} above the true {:.17e} on analytic faces by {:.3e} ({:.1} tolerances)",
                        t.faces,
                        d - t.faces,
                        (d - t.faces) / tol
                    ));
                } else if case.analytic {
                    wrong.push(format!(
                        "distance: {d:.17e} above the true {:.17e} on analytic faces (by {:.3e})",
                        t.faces,
                        d - t.faces
                    ));
                } else {
                    notes.push(format!(
                        "distance: {d:.6e} short of the true {:.6e} on a spline (documented)",
                        t.faces
                    ));
                }
            }
        }
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
    lines.extend(notes);
    Some((verdict, lines.join("; ")))
}

/// The cases of `kind` from `seed`, `count` of them that the engine
/// builds (at most four draws per case).
fn sweep(kind: Kind, seed: u64, count: usize) -> Tally {
    let mut rng = Rng(seed ^ (kind as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut tally = Tally::default();
    let mut index = 0;
    let mut draws = 0;
    while index < count && draws < 8 * count {
        draws += 1;
        let Some(case) = case(&mut rng, kind, index) else {
            continue;
        };
        let started = std::time::Instant::now();
        let Some((verdict, why)) = judge(&case) else {
            continue;
        };
        let ms = started.elapsed().as_millis();
        if why.contains("inexact") {
            tally.inexact += 1;
            println!("I {} {}: {why}", case.id, case.what);
        }
        let known = KNOWN.iter().any(|(kk, i, _)| *kk == kind && *i == index);
        let line = format!("{}: {why} [{ms} ms]", case.what);
        if verdict != Verdict::Right || std::env::var("HUNT_VERBOSE").is_ok() {
            println!(
                "{} {} {}{}",
                verdict.letter(),
                case.id,
                line,
                if known { " (known)" } else { "" }
            );
        }
        tally.add(&case.id, verdict, line);
        index += 1;
    }
    tally
}

/// Cases found wrong, by kind and index in the gate's sweep (seed
/// [`GATE_SEED`]), with what is wrong. A fix takes them off.
///
/// None at present. The ten it held until 2026-11-14 (BallCone 0 and 3,
/// ConeBigBall 0 to 2 and 4 to 8) were each a cone's face that holds its
/// apex: the proof on the exact faces covered only a band of it at its
/// base, so `atLeast` was the distance found, wherever the settling
/// stopped (see [`a_ball_sunk_into_a_slender_cone_is_an_overlap`]).
const KNOWN: [(Kind, usize, &str); 0] = [];

const GATE_SEED: u64 = 0x5EED_C4EC_0001;

/// The gate's part: a fixed set of every kind, held to the verdicts.
fn hold(kind: Kind, count: usize) {
    let tally = sweep(kind, GATE_SEED, count);
    tally.print(&format!("{kind:?}"));
    for (id, letter, what) in &tally.notes {
        let index: usize = id.rsplit(' ').next().unwrap().parse().unwrap();
        let known = KNOWN.iter().any(|(k, i, _)| *k == kind && *i == index);
        assert!(
            *letter == 'D' || (known && *letter == 'W'),
            "{id} {letter}: {what}"
        );
    }
    for (k, i, _) in KNOWN.iter().filter(|(k, ..)| *k == kind) {
        let id = format!("{k:?} {i}");
        assert!(
            tally.notes.iter().any(|(n, l, _)| *n == id && *l == 'W'),
            "{id} is listed as known wrong but is not: take it off KNOWN"
        );
    }
}

const GATE_CASES: usize = 10;

#[test]
fn balls() {
    hold(Kind::BallBall, GATE_CASES);
    hold(Kind::BallInBall, GATE_CASES);
}

#[test]
fn balls_and_boxes() {
    hold(Kind::BallBox, GATE_CASES);
    hold(Kind::BallInBox, GATE_CASES);
}

#[test]
fn balls_and_cylinders() {
    hold(Kind::BallCylinder, GATE_CASES);
}

#[test]
fn cylinders() {
    hold(Kind::CylParallel, GATE_CASES);
    hold(Kind::CylSkew, GATE_CASES);
    hold(Kind::PinHole, GATE_CASES);
}

#[test]
fn rings() {
    hold(Kind::TorusPlane, GATE_CASES);
    hold(Kind::TorusBall, GATE_CASES);
}

#[test]
fn cones() {
    hold(Kind::ConePlane, GATE_CASES);
}

#[test]
fn boxes() {
    hold(Kind::BoxBox, GATE_CASES);
    hold(Kind::ThinPlates, GATE_CASES);
}

#[test]
fn hollows() {
    hold(Kind::BallInHollow, GATE_CASES);
}

#[test]
fn poles_pins_and_sizes() {
    hold(Kind::BallCone, GATE_CASES);
    hold(Kind::TorusPin, GATE_CASES);
    hold(Kind::BigBall, GATE_CASES / 2);
    hold(Kind::Mismatch, GATE_CASES / 2);
}

#[test]
fn a_vase() {
    hold(Kind::Vase, GATE_CASES / 2);
}

#[test]
fn lumps() {
    hold(Kind::Lumps, GATE_CASES);
}

#[test]
fn big_balls_by_cones() {
    hold(Kind::ConeBigBall, GATE_CASES);
}

#[test]
fn splines() {
    hold(Kind::SplineBall, GATE_CASES);
    hold(Kind::SplineSpline, GATE_CASES / 2);
}

fn wide(kinds: &[Kind]) {
    let count: usize = std::env::var("HUNT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let seed: u64 = std::env::var("HUNT_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_C4EC_1000);
    let mut all = Tally::default();
    for &kind in kinds {
        let tally = sweep(kind, seed, count);
        println!("{}", tally.summary(&format!("{kind:?}")));
        all.merge(tally);
    }
    println!("{}", all.summary("all"));
}

#[test]
#[ignore]
fn wide_balls() {
    wide(&[
        Kind::BallBall,
        Kind::BallInBall,
        Kind::BallBox,
        Kind::BallInBox,
    ]);
}

#[test]
#[ignore]
fn wide_cylinders() {
    wide(&[
        Kind::BallCylinder,
        Kind::CylParallel,
        Kind::CylSkew,
        Kind::PinHole,
    ]);
}

#[test]
#[ignore]
fn wide_rings_and_cones() {
    wide(&[Kind::TorusPlane, Kind::TorusBall, Kind::ConePlane]);
}

#[test]
#[ignore]
fn wide_boxes() {
    wide(&[Kind::BoxBox, Kind::ThinPlates, Kind::BallInHollow]);
}

#[test]
#[ignore]
fn wide_splines() {
    wide(&[Kind::SplineBall, Kind::SplineSpline]);
}

#[test]
#[ignore]
fn wide_poles_pins_and_sizes() {
    wide(&[
        Kind::BallCone,
        Kind::TorusPin,
        Kind::BigBall,
        Kind::Mismatch,
    ]);
}

#[test]
#[ignore]
fn wide_vases() {
    wide(&[Kind::Vase]);
}

#[test]
#[ignore]
fn wide_lumps() {
    wide(&[Kind::Lumps]);
}

#[test]
#[ignore]
fn wide_big_balls_by_cones() {
    wide(&[Kind::ConeBigBall]);
}

/// One case in full, for a reproducer: `HUNT_KIND` (as in [`Kind`]'s
/// names), `HUNT_INDEX`, `HUNT_SEED` (default the gate's): the steps as a
/// `craft3d-cli run` script on standard output, then every answer.
#[test]
#[ignore]
fn explain() {
    let Ok(kind) = std::env::var("HUNT_KIND") else {
        return;
    };
    let kind = *KINDS
        .iter()
        .find(|k| format!("{k:?}") == kind)
        .expect("a kind");
    let index: usize = std::env::var("HUNT_INDEX").unwrap().parse().unwrap();
    let seed: u64 = std::env::var("HUNT_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(GATE_SEED);
    let mut rng = Rng(seed ^ (kind as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut built = 0;
    for _ in 0..8 * (index + 1) {
        let Some(case) = case(&mut rng, kind, built) else {
            continue;
        };
        let mut s = document(case.k);
        if build_case(&mut s, &case.steps).is_err() {
            continue;
        }
        if built < index {
            built += 1;
            continue;
        }
        println!("{}: {}", case.id, case.what);
        println!("truth {:?}", case.truth);
        let mut script = vec![json!(["file.new", {"units": "mm", "tolerance": 1e-6 * case.k}])];
        script.extend(case.steps.iter().map(|(id, p)| json!([id, p])));
        println!("{}", serde_json::to_string(&script).unwrap());
        for (id, p) in [
            ("measure.clearance", json!({"from": "A", "to": "B"})),
            ("measure.clearance", json!({"from": "B", "to": "A"})),
            ("measure.distance", json!({"from": "A", "to": "B"})),
            ("measure.distance", json!({"from": "B", "to": "A"})),
        ] {
            println!("{id} {p}: {:?}", run(&mut s, id, &p));
        }
        return;
    }
}

/// What the clearance of `A` to `B` (and the other way) and an assertion
/// `min` say, and where the ray from `c` along `dir` meets `A`.
struct Seen {
    at_least: [f64; 2],
    value: [f64; 2],
    status: String,
    hit: f64,
}

fn seen(s: &mut craft3d_engine::Session, c: V, dir: V, min: f64) -> Seen {
    let one = |s: &mut craft3d_engine::Session, a: &str, b: &str| {
        let Ran::Ok(out) = run(s, "measure.clearance", &json!({"from": a, "to": b})) else {
            panic!("clearance {a} {b}");
        };
        (
            out["atLeast"].as_f64().unwrap(),
            out["clearance"].as_f64().unwrap(),
        )
    };
    let (l1, v1) = one(s, "A", "B");
    let (l2, v2) = one(s, "B", "A");
    let Ran::Ok(ray) = run(
        s,
        "measure.ray",
        &json!({"from": mm3(c), "direction": num3(dir), "of": "A"}),
    ) else {
        panic!("ray");
    };
    let Ran::Ok(check) = run(
        s,
        "assert.add",
        &json!({"name": "gap", "measure": "clearance", "from": "A", "to": "B", "min": mm(min)}),
    ) else {
        panic!("assertion");
    };
    Seen {
        at_least: [l1, l2],
        value: [v1, v2],
        status: check["status"].as_str().unwrap().to_string(),
        hit: ray["hits"][0]["distance"].as_f64().unwrap(),
    }
}

/// The frames a pinned case is held in: the world, and turned about a
/// random axis and moved a thousand sizes off.
fn frames(rng: &mut Rng, k: f64) -> [Motion; 2] {
    [
        Motion::NONE,
        Motion {
            turn: Turn::random(rng),
            shift: rng.direction() * (1000.0 * k),
        },
    ]
}

/// Found by this hunt (2026-11-14) and repaired the same day: a ball of
/// radius 200 beside a slender cone (radius 0.3, height 20, at the origin
/// along z), 3e-6 (three tolerances) from its side at half its height.
/// The true clearance is 3e-6: the ball's centre is 200 + 3e-6 from a
/// point of the side along its normal (closed form, and the kernel's
/// exact ray from the centre along that normal meets the side at
/// 200.000003), and no point of the cone is nearer (in the meridian plane
/// the side is the segment nearest the centre, and the foot lies within
/// it). `measure.clearance` answered clear at 4.12e-4 with `atLeast`
/// 4.12e-4 (and 1.94e-4 the other way): it proved 137 times the true gap,
/// so an assertion `min 0.0002 mm` passed. `atLeast` was the distance
/// found, which the settling leaves far above the gap here; the proof on
/// the exact faces looked at a band of the cone at its base only (a
/// cone's face that holds its apex has no edge there) and did not bring
/// it down. Now, as for a cylinder of the cone's radius in the same
/// place: what is proven is never above the gap, and the assertion of 200
/// tolerances fails; at three scales, in the world and turned and moved
/// far off.
#[test]
fn a_ball_by_a_slender_cone_is_proven_no_farther_than_it_is() {
    let mut rng = Rng(GATE_SEED ^ 0xC0E1);
    for k in [1e-3, 1.0, 1e6] {
        let (r, h, rb, g, t): (f64, f64, f64, f64, f64) =
            (0.3 * k, 20.0 * k, 200.0 * k, 3e-6 * k, 0.5);
        let tol = 1e-6 * k;
        for motion in frames(&mut rng, k) {
            for cone in [true, false] {
                let (foot, n) = if cone {
                    let sl = h.phypot(r);
                    ((r * (1.0 - t), h * t), (h / sl, r / sl))
                } else {
                    ((r, h * t), (1.0, 0.0))
                };
                let p = (foot.0 + n.0 * (rb + g), foot.1 + n.1 * (rb + g));
                let c = V::new(p.0 * 0.6, p.0 * 0.8, p.1);
                let dir = V::new(-n.0 * 0.6, -n.0 * 0.8, -n.1);
                let label = format!(
                    "{} at scale {k:e}, {}",
                    if cone { "the cone" } else { "the cylinder" },
                    motion.describe()
                );
                let mut s = document(k);
                let a = if cone {
                    (
                        "feature.cone",
                        json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                    )
                } else {
                    (
                        "feature.cylinder",
                        json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                    )
                };
                build(
                    &mut s,
                    &[
                        a,
                        (
                            "feature.sphere",
                            json!({"name": "B0", "radius": mm(rb), "position": mm3(c)}),
                        ),
                        motion.step("A0", "A"),
                        motion.step("B0", "B"),
                    ],
                )
                .unwrap_or_else(|e| panic!("{label}: {e:?}"));
                let got = seen(&mut s, motion.apply(c), motion.turn.apply(dir), 200.0 * tol);
                // The second means: a point of A's side at `hit` from the
                // centre, so the bodies are at most `hit − rb` apart.
                let gap = got.hit - rb;
                let eps = 1e-13 * (motion.reach() + 1000.0 * k);
                assert!(
                    (gap - g).abs() < 1e-3 * tol + eps,
                    "{label}: the ray meets the side at {}",
                    got.hit
                );
                assert!(
                    got.at_least.iter().all(|&l| l <= gap + eps) && got.status == "failed",
                    "{label}: atLeast {:?}, clearance {:?}, the assertion {} (gap {gap:e})",
                    got.at_least,
                    got.value,
                    got.status
                );
            }
        }
    }
}

/// Found by this hunt (2026-11-14) and repaired the same day, the worst
/// of it: the same slender cone (radius 0.3, height 20) and a ball of
/// radius 200 sunk 1e-4 (a hundred tolerances) into its side at half its
/// height. They share volume: the ball's centre is 200 − 1e-4 from a
/// point of the side (closed form), and the kernel's intersection (the
/// measure `overlap`, which no distance code takes part in) finds a
/// common volume of about 1.7e-7. `measure.clearance` answered clear,
/// 7.6e-4 apart, proven at least 7.6e-4, and an assertion of a clearance
/// of at least 0 passed. Sunk 3e-6 and 1e-5 it was answered clear too. Of
/// 192 such pairs (cones from flat to slender, balls of radius 0.5 to
/// 500, sunk 3 to 1000 tolerances at a quarter, half and three quarters
/// of the height) 13 were answered clear, the worst a ball of radius 500
/// sunk 1e-3 (a thousand tolerances) into the slender cone, proven 0.0377
/// apart.
///
/// Why: the proof on the exact faces started a face of a cone from the
/// box of its heights along the axis taken from its edges
/// (`verify_bound.rs`, `face_root`: "the height along the axis over a
/// cylinder or a cone, is least and most on the face's boundary"); a
/// cone's face that holds its apex has no edge there (feature.cone's side
/// is bounded by its base circle alone), so the box held a band at the
/// base and none of the rest of the face. What the proof then bounded was
/// that band, which lies farther than the place found, so `atLeast` was
/// the place found, however far the settling stopped above the truth;
/// with the bound above twice the tolerance the kernel was not asked, and
/// each body's points lie outside the other, so the pair was clear.
///
/// Now, at three scales, in the world and turned and moved far off, sunk
/// 10, 100 and 1000 tolerances: never clear either way round and never
/// with a value, an overlap wherever the kernel tells (always in the
/// world), and an assertion of a clearance of at least 0 never passes.
#[test]
fn a_ball_sunk_into_a_slender_cone_is_an_overlap() {
    let mut rng = Rng(GATE_SEED ^ 0xC0E2);
    for k in [1e-3, 1.0, 1e6] {
        for (far, motion) in frames(&mut rng, k).into_iter().enumerate() {
            for sunk in [1e-5, 1e-4, 1e-3] {
                let (r, h, rb, g, t): (f64, f64, f64, f64, f64) =
                    (0.3 * k, 20.0 * k, 200.0 * k, -sunk * k, 0.5);
                let sl = h.phypot(r);
                let (foot, n) = ((r * (1.0 - t), h * t), (h / sl, r / sl));
                let p = (foot.0 + n.0 * (rb + g), foot.1 + n.1 * (rb + g));
                let c = V::new(p.0 * 0.6, p.0 * 0.8, p.1);
                let label = format!("sunk {sunk:e} at scale {k:e}, {}", motion.describe());
                let mut s = document(k);
                build(
                    &mut s,
                    &[
                        (
                            "feature.cone",
                            json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                        ),
                        (
                            "feature.sphere",
                            json!({"name": "B0", "radius": mm(rb), "position": mm3(c)}),
                        ),
                        motion.step("A0", "A"),
                        motion.step("B0", "B"),
                    ],
                )
                .unwrap_or_else(|e| panic!("{label}: {e:?}"));
                // The second means: the kernel's common volume (where the
                // ball is deep enough for it to tell).
                if far == 0 && sunk >= 1e-4 {
                    let Ran::Ok(shared) = run(
                        &mut s,
                        "assert.add",
                        &json!({"name": "overlap", "measure": "overlap", "of": ["A", "B"], "is": false}),
                    ) else {
                        panic!("{label}: overlap");
                    };
                    let volume: f64 = shared["detail"]["overlaps"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|o| o["volume"].as_f64())
                        .sum();
                    assert!(volume > 1e-8 * k * k * k, "{label}: {shared}");
                }
                for (a, b) in [("A", "B"), ("B", "A")] {
                    match run(&mut s, "measure.clearance", &json!({"from": a, "to": b})) {
                        Ran::Ok(clear) => assert!(
                            clear["pairs"][0]["status"] == "overlap"
                                && clear["clearance"].is_null()
                                && clear["atLeast"].is_null(),
                            "{label}: {a} to {b}: {clear}"
                        ),
                        // Declined with a reason: only far off.
                        Ran::Err(e) => assert!(far == 1, "{label}: {a} to {b}: {e}"),
                        Ran::Panic(e) => panic!("{label}: {a} to {b}: {e}"),
                    }
                }
                let Ran::Ok(apart) = run(
                    &mut s,
                    "assert.add",
                    &json!({"name": "apart", "measure": "clearance", "from": "A", "to": "B", "min": "0 mm"}),
                ) else {
                    panic!("{label}: assertion");
                };
                assert!(
                    apart["status"] == "failed" || (far == 1 && apart["status"] == "error"),
                    "{label}: {apart}"
                );
            }
        }
    }
}

/// Found by this hunt (2026-11-14), owed, lesser: a ball of radius
/// 6 999 630 (7000 times the scale) sunk 1 (a thousand tolerances of 1e-3)
/// into a block 20 000 × 20 000 × 5000, turned: their faces cross along
/// a circle of radius about 3700, so they are 0 apart, and the clearance
/// says overlap. `measure.distance` answers 0.301 (301 tolerances), with
/// `atLeast` 0, honestly: the distance between faces that cross is not
/// the 0 the command promises ("faces that touch or cross are 0 apart").
/// The same where a ring lies flat 3 tolerances deep in a plane (3.0
/// found) and in 7 of 120 such big balls. No proof is false here and no
/// assertion of a minimum passes on it; one of a maximum is not proven
/// rather than passed. When fixed the test fails: make it the right
/// answer.
#[test]
fn a_big_ball_sunk_into_a_block_is_given_a_distance_known() {
    let mut s = document(1e3);
    let turn = Motion {
        turn: Turn {
            axis: V::new(
                -0.27707504544073286,
                -0.8959734073866586,
                -0.3470750242382135,
            ),
            deg: 80.74034649170073,
        },
        shift: V::new(0.0, 0.0, 0.0),
    };
    build(
        &mut s,
        &[
            (
                "feature.block",
                json!({"name": "A0", "size": [mm(20000.0), mm(20000.0), mm(5000.0)]}),
            ),
            (
                "feature.sphere",
                json!({"name": "B0", "radius": mm(6999629.872468182), "position": [mm(11644.6167074468), mm(8916.159308371813), mm(7004628.872468182)]}),
            ),
            turn.step("A0", "A"),
            turn.step("B0", "B"),
        ],
    )
    .unwrap();
    let Ran::Ok(clear) = run(
        &mut s,
        "measure.clearance",
        &json!({"from": "A", "to": "B"}),
    ) else {
        panic!("clearance");
    };
    assert_eq!(clear["pairs"][0]["status"], "overlap", "{clear}");
    let Ran::Ok(d) = run(&mut s, "measure.distance", &json!({"from": "A", "to": "B"})) else {
        panic!("distance");
    };
    let (found, at_least) = (
        d["distance"].as_f64().unwrap(),
        d["atLeast"].as_f64().unwrap(),
    );
    assert!(at_least <= 0.0, "{d}");
    assert!(
        found > 0.1,
        "the crossing faces are given {found} now: if within the tolerance (1e-3) of 0, it is \
         fixed; make this the right answer"
    );
}

/// Found by the review of the cone's proof (2026-11-14), owed, known
/// wrong: **a pair that shares volume is answered clear, with nothing
/// proven, and an assertion of a clearance of at least 0 passes.** A ball
/// about 2500 times the body, sunk 3 to 10 tolerances into a cone's or a
/// cylinder's side:
/// - a cone of radius 4 and height 12 and a ball of radius 10 000 at
///   (9488.832971018304, 0, 3168.277657006102): its centre is 10 000 −
///   1e-5 from the side at half its height along the side's normal
///   (closed form), so its surface lies 1e-5 (10 tolerances) inside the
///   cone; `measure.clearance` answers clear, 1.097e-6, `atLeast` 0;
/// - a cylinder of radius 4 and height 10 from z = −5 and a ball of
///   radius 10 000 at (10003.999997, 0, 0.7), 3e-6 (3 tolerances) inside
///   its wall: clear the same way.
///
/// Nothing false is proven (`atLeast` is 0), but the status is false, and
/// `clearance min "0 mm"` and `overlap is false` pass on bodies that
/// overlap. The cause is not in the distance or its proof: where nothing
/// is proven beyond twice the tolerance, `verify_cmds::clearance_status`
/// asks the kernel (`craft3d_kernel::boolean::contact`), which answers
/// Clear for such a pair (`feature.boolean` intersect of it leaves
/// nothing, `document.interference` says clear): a common volume a few
/// tolerances deep under a ball thousands of times the body. Balls of
/// 0.5 to 200 sunk 3 to 200 tolerances are answered overlap (168 of 168
/// in the review). The same before and after the repair of the cone's
/// proof (before it, the cone's case was clear with a false proof).
/// When the kernel's contact is fixed this test fails: make it the right
/// answer (an overlap; both assertions failed).
#[test]
fn a_huge_ball_sunk_a_few_tolerances_into_a_side_is_answered_clear_known() {
    let tol = 1e-6;
    let (rb, r): (f64, f64) = (10_000.0, 4.0);
    // The cone's case by its closed form: from the side's point at half
    // height, along its normal, 1e-5 short of the ball's radius.
    let slant = 12.0_f64.phypot(r);
    let (foot, n) = ((2.0, 6.0), (12.0 / slant, r / slant));
    let by_form = V::new(foot.0 + n.0 * (rb - 1e-5), 0.0, foot.1 + n.1 * (rb - 1e-5));
    assert!(
        (by_form.x - 9488.832971018304).abs() < 1e-8
            && (by_form.z - 3168.277657006102).abs() < 1e-8,
        "{by_form:?}"
    );
    // (the body, the ball's centre, how deep its surface is in the body)
    let cases = [
        (
            (
                "feature.cone",
                json!({"name": "A", "radius": mm(r), "height": mm(12.0)}),
            ),
            V::new(9488.832971018304, 0.0, 3168.277657006102),
            1e-5,
        ),
        (
            (
                "feature.cylinder",
                json!({"name": "A", "radius": mm(r), "height": mm(10.0), "position": mm3(V::new(0.0, 0.0, -5.0))}),
            ),
            V::new(10003.999997, 0.0, 0.7),
            rb + r - 10003.999997,
        ),
    ];
    for (body, centre, depth) in cases {
        let what = body.0;
        assert!(depth > 2.5 * tol && depth < 11.0 * tol, "{what}: {depth:e}");
        let mut s = document(1.0);
        build(
            &mut s,
            &[
                body,
                (
                    "feature.sphere",
                    json!({"name": "B", "radius": mm(rb), "position": mm3(centre)}),
                ),
            ],
        )
        .unwrap();
        let Ran::Ok(clear) = run(
            &mut s,
            "measure.clearance",
            &json!({"from": "A", "to": "B"}),
        ) else {
            panic!("{what}: clearance");
        };
        let Ran::Ok(apart) = run(
            &mut s,
            "assert.add",
            &json!({"name": "apart", "measure": "clearance", "from": "A", "to": "B", "min": "0 mm"}),
        ) else {
            panic!("{what}: assertion");
        };
        let Ran::Ok(none) = run(
            &mut s,
            "assert.add",
            &json!({"name": "none", "measure": "overlap", "of": ["A", "B"], "is": false}),
        ) else {
            panic!("{what}: overlap");
        };
        // Never a proof above the truth, known or not.
        let at_least = clear["atLeast"].as_f64().unwrap_or(0.0);
        assert!(
            at_least <= 0.0,
            "{what}: proven {at_least:e} apart: {clear}"
        );
        assert!(
            clear["pairs"][0]["status"] == "clear"
                && apart["status"] == "passed"
                && none["status"] == "passed",
            "{what}, sunk {depth:e}, is answered otherwise now: {clear}; at least 0: {}; no \
             overlap: {}: if an overlap and both failed, it is fixed; make this the right answer",
            apart["status"],
            none["status"]
        );
    }
}
