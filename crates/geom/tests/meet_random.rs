//! Completeness on seeded random pairs of every kind combination.
//!
//! One surface is sampled on a fine grid inside the box and the signed
//! distance to the other is taken at every node, from formulas written
//! here (the implicit equations of the analytic kinds; the foot point from
//! `project` for freeform surfaces). Wherever two neighbouring nodes
//! differ in sign the surfaces cross between them, so a returned curve
//! must pass there; and wherever a returned curve passes, the nodes around
//! it must differ in sign. Run with `--nocapture` for the table of how
//! many cases were solved and refused per pair, and the slowest of each.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

use craft3d_math::Portable;
mod common;

use common::*;
use craft3d_geom::intersect::{Meeting, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{BezierPatch, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Vec3};
use craft3d_testkit::parallel::map_parallel;
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::time::Instant;

const KINDS: [&str; 7] = [
    "plane", "cylinder", "cone", "sphere", "torus", "bezier", "nurbs",
];
const HALF: f64 = 6.0;

fn random_surface(kind: usize, rng: &mut Rng) -> Surface {
    let net = |rng: &mut Rng, n: usize, pitch: f64| {
        let f = rng.frame(1.5);
        let mut ctrl = Vec::new();
        let middle = 0.5 * (n as f64 - 1.0);
        for i in 0..n {
            for j in 0..n {
                let z = rng.range(-0.9, 0.9);
                ctrl.push(f.point((i as f64 - middle) * pitch, (j as f64 - middle) * pitch, z));
            }
        }
        ctrl
    };
    match kind {
        0 => plane(rng.frame(1.5)),
        1 => cylinder(rng.frame(2.0), rng.range(0.5, 2.5)),
        2 => {
            let side = if rng.next().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            cone(
                rng.frame(2.0),
                rng.range(0.0, 1.5),
                side * rng.range(0.2, 0.9),
            )
        }
        3 => sphere(rng.frame(2.0), rng.range(1.0, 3.0)),
        4 => {
            let major = rng.range(1.5, 3.0);
            torus(rng.frame(2.0), major, rng.range(0.3, 0.7 * major))
        }
        5 => Surface::Bezier(BezierPatch::new(3, 3, net(rng, 4, 1.6)).unwrap()),
        _ => {
            let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
            let ctrl = net(rng, 5, 1.3);
            let weights = (0..25).map(|_| rng.range(0.6, 1.6)).collect();
            Surface::Nurbs(
                NurbsSurface::new(3, 3, knots.clone(), knots, 5, 5, ctrl, weights).unwrap(),
            )
        }
    }
}

/// The signed distance from `p` to a surface (for a cone a function with
/// the same zero set and sign). `None` past the edge of a freeform patch.
fn implicit(surface: &Surface, p: Vec3) -> Option<f64> {
    match surface {
        Surface::Plane(s) => Some(s.frame().local(p).2),
        Surface::Cylinder(s) => {
            let (x, y, _) = s.frame().local(p);
            Some(x.phypot(y) - s.radius())
        }
        Surface::Cone(s) => {
            let (x, y, z) = s.frame().local(p);
            let radius = s.radius() + z * s.half_angle().ptan();
            // Past the apex the radius is negative and this stays positive:
            // there is no surface there, everything is outside.
            Some(x.phypot(y) - radius)
        }
        Surface::Sphere(s) => Some(p.distance(s.frame().origin()) - s.radius()),
        Surface::Torus(s) => {
            let (x, y, z) = s.frame().local(p);
            Some((x.phypot(y) - s.radii().0).phypot(z) - s.radii().1)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let ((u0, u1), (v0, v1)) = surface.domain();
            let (u, v) = surface.project(p).ok()?;
            let edge = |x: f64, a: f64, b: f64| x - a <= 1e-9 * (b - a) || b - x <= 1e-9 * (b - a);
            if edge(u, u0, u1) || edge(v, v0, v1) {
                return None;
            }
            Some((p - surface.eval(u, v)).dot(surface.normal(u, v)?))
        }
    }
}

/// The rectangle of parameters of a surface that covers the box.
fn grid_domain(surface: &Surface) -> ((f64, f64), (f64, f64)) {
    let far = 3.0 * HALF;
    match surface {
        Surface::Plane(_) => ((-far, far), (-far, far)),
        Surface::Cylinder(_) => ((0.0, TAU), (-far, far)),
        Surface::Cone(c) => {
            let apex = c.apex_v();
            if c.half_angle() > 0.0 {
                ((0.0, TAU), (apex, apex + 2.0 * far))
            } else {
                ((0.0, TAU), (apex - 2.0 * far, apex))
            }
        }
        _ => surface.domain(),
    }
}

/// Checks the meeting of `a` and `b` against the signs of `b`'s distance
/// on a grid over `a`. Returns how many sign changes were seen.
fn check_complete(m: &Meeting, a: &Surface, b: &Surface, within: &Bbox3, label: &str) -> usize {
    let freeform = matches!(b, Surface::Bezier(_) | Surface::Nurbs(_));
    let n = if freeform { 56 } else { 220 };
    let ((u0, u1), (v0, v1)) = grid_domain(a);
    let at = |i: usize, j: usize| {
        (
            u0 + (u1 - u0) * i as f64 / n as f64,
            v0 + (v1 - v0) * j as f64 / n as f64,
        )
    };
    let shrunk = {
        let (lo, hi) = within.corners().unwrap();
        let d = Vec3::new(0.05, 0.05, 0.05);
        Bbox3::from_points([lo + d, hi - d])
    };
    // Per node: the point and the sign, where the node counts.
    let mut nodes: Vec<Option<(Vec3, bool)>> = Vec::with_capacity((n + 1) * (n + 1));
    for i in 0..=n {
        for j in 0..=n {
            let (u, v) = at(i, j);
            let p = a.eval(u, v);
            nodes.push(if shrunk.contains(p, 0.0) {
                implicit(b, p).map(|f| (p, f >= 0.0))
            } else {
                None
            });
        }
    }
    let node = |i: usize, j: usize| nodes[i * (n + 1) + j];
    let lines = Lines::of(m, 3000);
    let mut changes = 0;
    let mut changed = vec![false; (n + 1) * (n + 1)];
    for i in 0..=n {
        for j in 0..=n {
            for (k, l) in [(i + 1, j), (i, j + 1)] {
                if k > n || l > n {
                    continue;
                }
                let (Some((p, sp)), Some((q, sq))) = (node(i, j), node(k, l)) else {
                    continue;
                };
                // A freeform foot point can jump between sheets; a real
                // crossing has the nodes close to the other surface.
                if sp == sq || p.distance(q) > 0.5 {
                    continue;
                }
                if freeform {
                    let near = |x: Vec3| implicit(b, x).is_some_and(|f| f.abs() <= p.distance(q));
                    if !(near(p) && near(q)) {
                        continue;
                    }
                }
                changes += 1;
                changed[i * (n + 1) + j] = true;
                changed[k * (n + 1) + l] = true;
                let gap = lines.distance(p.lerp(q, 0.5));
                assert!(
                    gap <= p.distance(q) + 1e-4,
                    "{label}: the surfaces cross between {p:?} and {q:?}, {gap} from any curve of {}",
                    m.curves.len()
                );
            }
        }
    }
    // The other way: around every point of a curve the sign changes.
    for curve in &m.curves {
        if curve.tangent {
            continue;
        }
        for p in samples(curve, 40) {
            if !shrunk.contains(p, -0.3) {
                continue;
            }
            let (Some(na), Some(nb)) = (normal_at(a, p), normal_at(b, p)) else {
                continue;
            };
            if na.cross(nb).length() < 0.05 {
                continue; // Too flat a crossing for this grid to see.
            }
            let (u, v) = a.project(p).unwrap();
            let (u, v) = if matches!(
                a,
                Surface::Cone(_) | Surface::Cylinder(_) | Surface::Sphere(_) | Surface::Torus(_)
            ) {
                (
                    u.rem_euclid(TAU),
                    if matches!(a, Surface::Torus(_)) {
                        v.rem_euclid(TAU)
                    } else {
                        v
                    },
                )
            } else {
                (u, v)
            };
            let cell =
                |x: f64, lo: f64, hi: f64| (((x - lo) / (hi - lo)) * n as f64).floor() as i64;
            let (ci, cj) = (cell(u, u0, u1), cell(v, v0, v1));
            let mut seen = false;
            let mut blind = false;
            for i in ci - 2..=ci + 3 {
                for j in cj - 2..=cj + 3 {
                    if i < 0 || j < 0 || i > n as i64 || j > n as i64 {
                        blind = true;
                        continue;
                    }
                    let index = i as usize * (n + 1) + j as usize;
                    seen |= changed[index];
                    blind |= nodes[index].is_none();
                }
            }
            assert!(
                seen || blind,
                "{label}: a curve passes {p:?} but the surfaces do not cross there"
            );
        }
    }
    changes
}

#[derive(Default)]
struct Tally {
    crossing: usize,
    apart: usize,
    refused: BTreeMap<String, usize>,
    slowest_ms: f64,
    curves: usize,
}

#[test]
fn every_kind_pair_is_complete_on_random_cases() {
    let within = cube(Vec3::ZERO, HALF);
    let t = tol(1.0);
    let mut pairs = Vec::new();
    for i in 0..KINDS.len() {
        for j in i..KINDS.len() {
            pairs.push((i, j));
        }
    }
    // Every pair has its own seed, so the cases are the same whatever the
    // thread count; the pairs run side by side and the table keeps their
    // order.
    let table: Vec<(String, Tally)> = map_parallel(pairs, |(i, j)| {
        // NURBS evaluation is slow: fewer cases where it dominates.
        let cases = match (i >= 5, j) {
            (true, 6) => 2,
            (true, _) => 4,
            (false, 6) => 5,
            (false, 5) => 8,
            _ => 12,
        };
        let mut tally = Tally::default();
        let mut rng = Rng(0x3D_C4AF + 1000 * i as u64 + j as u64);
        for case in 0..cases {
            let (a, b) = (random_surface(i, &mut rng), random_surface(j, &mut rng));
            let label = format!("{} / {} case {case}", KINDS[i], KINDS[j]);
            let clock = Instant::now();
            let found = meet_surfaces(&a, &b, &within, &t);
            let ms = clock.elapsed().as_secs_f64() * 1e3;
            tally.slowest_ms = tally.slowest_ms.max(ms);
            assert_eq!(found, meet_surfaces(&b, &a, &within, &t), "{label}: order");
            if case % 4 == 0 {
                assert_eq!(found, meet_surfaces(&a, &b, &within, &t), "{label}: repeat");
            }
            let m = match found {
                Ok(SurfaceMeeting::Meets(m)) => m,
                Ok(SurfaceMeeting::None) => Meeting {
                    curves: Vec::new(),
                    points: Vec::new(),
                },
                Ok(SurfaceMeeting::Coincident { .. }) => panic!("{label}: coincident"),
                Err(e) => {
                    *tally.refused.entry(e.to_string()).or_default() += 1;
                    continue;
                }
            };
            check_meeting(&m, &a, &b, &within, &t);
            // Grid over one surface, then over the other.
            // A freeform surface against an analytic one carries the
            // grid: the analytic distance is exact and cheap.
            let (g, o) = match (i >= 5, j >= 5) {
                (false, true) => (&b, &a),
                (true, false) => (&a, &b),
                _ if case % 2 == 0 => (&a, &b),
                _ => (&b, &a),
            };
            let changes = check_complete(&m, g, o, &within, &label);
            if m.curves.is_empty() {
                assert_eq!(changes, 0, "{label}");
                tally.apart += 1;
            } else {
                tally.crossing += 1;
                tally.curves += m.curves.len();
            }
        }
        (format!("{} / {}", KINDS[i], KINDS[j]), tally)
    });
    eprintln!("pair                  crossing  apart  refused  curves  slowest ms");
    let mut refused_total = 0;
    let mut total = 0;
    for (pair, tally) in &table {
        let refused: usize = tally.refused.values().sum();
        refused_total += refused;
        total += tally.crossing + tally.apart + refused;
        eprintln!(
            "{pair:<22}{:>8}{:>7}{:>9}{:>8}{:>12.1}",
            tally.crossing, tally.apart, refused, tally.curves, tally.slowest_ms
        );
        for (reason, count) in &tally.refused {
            eprintln!("    {count} x {reason}");
        }
    }
    eprintln!("{refused_total} of {total} cases refused");
    // Refusals are allowed, but not as the rule.
    assert!(
        refused_total * 10 <= total,
        "{refused_total} of {total} refused"
    );
    assert!(
        table.iter().all(|(_, t)| t.crossing > 0),
        "a pair never crossed"
    );
}
