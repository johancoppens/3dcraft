//! Clusters of thin tubes (found 2026-10-22 by bool-speed-3 while timing
//! the covers): four patterns of five tubes (radius 10, wall 0.3125,
//! length 15) turned a fifth of a turn each about an upright axis, the
//! patterns 3 along x and 1.7 along y apart, united one after the other.
//! The third union was refused, "an imprinted face could not be divided
//! into regions; this is a bug": a tube's bore had a hole whose edge is a
//! spline (where an earlier tube crossed it), and the outer wall of a tube
//! of the next pattern, parallel to the bore and less than half a unit off
//! its axis, meets the bore at a few degrees along lines that cross that
//! spline. The spline is only within the tolerance of the curve it stands
//! for, so where it meets the outer wall slid along the wall by several
//! tolerances; the cut there was further than the tolerance from the line
//! and dropped, and the line ran on across the hole. Such a hit is now
//! taken to where the three surfaces meet (`boolean/corner.rs`,
//! docs/design/booleans.md › *A spline edge that meets a surface at a
//! shallow angle*).
//!
//! Held to an independent reference: the volume of a union of tubes whose
//! axes lie in one horizontal plane, integrated over the height from
//! horizontal slices, each a union of rectangles whose area is exact
//! (`reference_volume`; the kernel agrees to 1e-8), and seeded points
//! against exact membership of the union; plus the full check, the
//! boolean's own check of every step, a watertight mesh, three scales and
//! bit-identical repeats.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{BooleanOp, boolean, contains_points};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::Body;
use std::f64::consts::TAU;

const OUTER: f64 = 10.0;
const BORE: f64 = 9.6875;
const LENGTH: f64 = 15.0;

/// A tube at scale 1: from `start` (at height 0) along the horizontal
/// unit vector `axis`, `LENGTH` long, between radii `BORE` and `OUTER`.
#[derive(Clone, Copy)]
struct Tube {
    start: Vec3,
    axis: Vec3,
}

impl Tube {
    /// Where `p` is along the axis, and how far from it.
    fn place(&self, p: Vec3) -> (f64, f64) {
        let d = p - self.start;
        let along = d.dot(self.axis);
        (along, (d - self.axis * along).length())
    }

    /// The body, times `k`: a cylinder less a longer, thinner one.
    fn body(&self, k: f64) -> Body {
        let up = Vec3::new(0.0, 0.0, 1.0);
        let start = self.start * k;
        let frame = Frame::new(start, self.axis, up).unwrap();
        let hole = Frame::new(start - self.axis * (LENGTH * k), self.axis, up).unwrap();
        boolean(
            &primitives::cylinder(&frame, OUTER * k, LENGTH * k).unwrap(),
            &primitives::cylinder(&hole, BORE * k, 3.0 * LENGTH * k).unwrap(),
            BooleanOp::Subtract,
            &tolerance(k),
        )
        .unwrap()
        .body
    }
}

/// Unites the tubes of each group one after the other, then the groups
/// one after the other, every step checked in full ([`solve`]).
fn unite(groups: &[Vec<Tube>], k: f64) -> Body {
    let mut all: Option<Body> = None;
    for group in groups {
        let mut one: Option<Body> = None;
        for tube in group {
            let body = tube.body(k);
            one = Some(match one {
                None => body,
                Some(so_far) => solve(&so_far, &body, BooleanOp::Union, k).body,
            });
        }
        let one = one.unwrap();
        all = Some(match all {
            None => one,
            Some(so_far) => solve(&so_far, &one, BooleanOp::Union, k).body,
        });
    }
    all.unwrap()
}

/// The volume of the union of the tubes (axes horizontal, at height 0),
/// by slices: at height `z` each tube is one rectangle (`|z|` between
/// `BORE` and `OUTER`: `|h| ≤ sqrt(OUTER² − z²)`, `h` the distance from the
/// axis seen from above) or two (below `BORE`: `sqrt(BORE² − z²) ≤ |h| ≤
/// sqrt(OUTER² − z²)`) over its length, so a slice of the union is a union
/// of rectangles whose area is exact ([`union_area`]). The area is
/// integrated over the height by adaptive Simpson's rule, in angles
/// (`z = BORE sin θ` below the bore's top, `z = OUTER sin φ` above it) in
/// which the widths of the rectangles have no square-root ends.
fn reference_volume(tubes: &[Tube]) -> f64 {
    let slice = |z: f64| -> f64 {
        let outer = (OUTER * OUTER - z * z).max(0.0).sqrt();
        let inner = (BORE * BORE - z * z).max(0.0).sqrt();
        let mut rects = Vec::with_capacity(2 * tubes.len());
        for t in tubes {
            let across = Vec3::new(-t.axis.y, t.axis.x, 0.0);
            let rect = |lo: f64, hi: f64| {
                let corner = |along: f64, side: f64| {
                    let p = t.start + t.axis * along + across * side;
                    (p.x, p.y)
                };
                // Counter-clockwise seen from above: along the axis, then
                // across it to the left.
                [
                    corner(0.0, lo),
                    corner(LENGTH, lo),
                    corner(LENGTH, hi),
                    corner(0.0, hi),
                ]
            };
            if z.abs() < BORE {
                rects.push(rect(inner, outer));
                rects.push(rect(-outer, -inner));
            } else {
                rects.push(rect(-outer, outer));
            }
        }
        union_area(&rects)
    };
    let below = |theta: f64| {
        let (s, c) = theta.psin_cos();
        slice(BORE * s) * BORE * c
    };
    let above = |phi: f64| {
        let (s, c) = phi.psin_cos();
        slice(OUTER * s) * OUTER * c
    };
    let top = (BORE / OUTER).pasin();
    let half = std::f64::consts::FRAC_PI_2;
    2.0 * (simpson(&below, 0.0, half, 1e-10) + simpson(&above, top, half, 1e-10))
}

/// The area of a union of convex quadrilaterals, each counter-clockwise:
/// by Green's theorem over the boundary of the union, which is made of
/// the pieces of their edges that no other one covers. (A piece on an
/// edge of another one that runs the same way is counted once, for the
/// first of them; one on an edge that runs the other way not at all.)
fn union_area(rects: &[[(f64, f64); 4]]) -> f64 {
    let mut twice = 0.0;
    let mut covered: Vec<(f64, f64)> = Vec::new();
    for (i, rect) in rects.iter().enumerate() {
        for e in 0..4 {
            let (p, q) = (rect[e], rect[(e + 1) % 4]);
            let d = (q.0 - p.0, q.1 - p.1);
            covered.clear();
            for (j, other) in rects.iter().enumerate() {
                if j == i {
                    continue;
                }
                // Where p + t d is in `other`: inside every edge of it.
                let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
                for f in 0..4 {
                    let (a, b) = (other[f], other[(f + 1) % 4]);
                    let edge = (b.0 - a.0, b.1 - a.1);
                    // Left of the edge is inside: edge × (x − a) ≥ 0.
                    let f0 = edge.0 * (p.1 - a.1) - edge.1 * (p.0 - a.0);
                    let f1 = edge.0 * d.1 - edge.1 * d.0;
                    if f1 == 0.0 {
                        let same_way = edge.0 * d.0 + edge.1 * d.1 > 0.0;
                        if f0 < 0.0 || (f0 == 0.0 && same_way && j > i) {
                            t1 = -1.0;
                        }
                    } else if f1 > 0.0 {
                        t0 = t0.max(-f0 / f1);
                    } else {
                        t1 = t1.min(-f0 / f1);
                    }
                }
                if t1 > t0 {
                    covered.push((t0, t1));
                }
            }
            covered.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut free: Vec<(f64, f64)> = Vec::new();
            let mut at = 0.0;
            for &(a, b) in &covered {
                if a > at {
                    free.push((at, a));
                }
                at = at.max(b);
            }
            if at < 1.0 {
                free.push((at, 1.0));
            }
            for (a, b) in free {
                let (x0, y0) = (p.0 + a * d.0, p.1 + a * d.1);
                let (x1, y1) = (p.0 + b * d.0, p.1 + b * d.1);
                twice += x0 * y1 - x1 * y0;
            }
        }
    }
    0.5 * twice
}

/// Adaptive Simpson's rule to `eps` (relative to the whole), started on
/// 64 pieces so that no feature of the integrand is stepped over.
fn simpson(f: &dyn Fn(f64) -> f64, a: f64, b: f64, eps: f64) -> f64 {
    /// A piece: its ends, the integrand at its ends and middle, and
    /// Simpson's rule over it.
    struct Piece {
        ends: (f64, f64),
        values: [f64; 3],
        whole: f64,
    }
    fn rec(f: &dyn Fn(f64) -> f64, piece: Piece, eps: f64, depth: u32) -> f64 {
        let ((a, b), [fa, fm, fb]) = (piece.ends, piece.values);
        let m = 0.5 * (a + b);
        let (flm, frm) = (f(0.5 * (a + m)), f(0.5 * (m + b)));
        let left = (m - a) / 6.0 * (fa + 4.0 * flm + fm);
        let right = (b - m) / 6.0 * (fm + 4.0 * frm + fb);
        let change = left + right - piece.whole;
        if depth == 0 || change.abs() <= 15.0 * eps {
            return left + right + change / 15.0;
        }
        let halves = [
            Piece {
                ends: (a, m),
                values: [fa, flm, fm],
                whole: left,
            },
            Piece {
                ends: (m, b),
                values: [fm, frm, fb],
                whole: right,
            },
        ];
        halves
            .into_iter()
            .map(|half| rec(f, half, 0.5 * eps, depth - 1))
            .sum()
    }
    let pieces = 64;
    let h = (b - a) / f64::from(pieces);
    let mut total = 0.0;
    for k in 0..pieces {
        let (x0, x1) = (a + h * f64::from(k), a + h * f64::from(k + 1));
        let values = [f(x0), f(0.5 * (x0 + x1)), f(x1)];
        let piece = Piece {
            ends: (x0, x1),
            values,
            whole: h / 6.0 * (values[0] + 4.0 * values[1] + values[2]),
        };
        total += rec(f, piece, eps / f64::from(pieces), 30);
    }
    total
}

/// Whether `p` (at scale 1) is in the union of the tubes; `None` within
/// `margin` of the boundary of any of them.
fn inside(tubes: &[Tube], p: Vec3, margin: f64) -> Option<bool> {
    let mut any = false;
    for t in tubes {
        let (along, r) = t.place(p);
        let near_wall = (r - BORE).abs() < margin || (r - OUTER).abs() < margin;
        let near_end = along.abs() < margin || (along - LENGTH).abs() < margin;
        let in_length = (-margin..=LENGTH + margin).contains(&along);
        let in_wall = (BORE - margin..=OUTER + margin).contains(&r);
        if (near_wall && in_length) || (near_end && in_wall) {
            return None;
        }
        any |= (0.0..=LENGTH).contains(&along) && (BORE..=OUTER).contains(&r);
    }
    Some(any)
}

/// Seeded points near the walls of the tubes, in the wall and on either
/// side of it.
fn points(tubes: &[Tube], count: usize, seed: u64) -> Vec<Vec3> {
    let mut rng = Rng(seed);
    let up = Vec3::new(0.0, 0.0, 1.0);
    (0..count)
        .map(|_| {
            let t = tubes[rng.below(tubes.len() as u64) as usize];
            let across = Vec3::new(-t.axis.y, t.axis.x, 0.0);
            let along = rng.between(-1.0, LENGTH + 1.0);
            let r = rng.between(BORE - 0.4, OUTER + 0.4);
            let (s, c) = rng.between(0.0, TAU).psin_cos();
            t.start + t.axis * along + (across * c + up * s) * r
        })
        .collect()
}

/// The union of the groups at three scales, held to the reference volume,
/// to the seeded points, to a watertight mesh, and made twice the same.
fn holds(groups: &[Vec<Tube>], what: &str) {
    let tubes: Vec<Tube> = groups.iter().flatten().copied().collect();
    let reference = reference_volume(&tubes);
    // The reference's adaptive rule is good to about 1e-8 of it.
    let slack = 1e-7 * reference;
    let at_one = unite(groups, 1.0);
    assert!(
        at_one == unite(groups, 1.0),
        "{what}: the same union twice is not the same body"
    );
    let measured = mass(&at_one);
    assert!(
        (measured.volume - reference).abs() <= slack,
        "{what}: volume {} against the reference {reference} ± {slack}",
        measured.volume
    );
    // Seeded points against exact membership, at scale 1.
    let seeded = points(&tubes, 600, 0x7b_e5);
    let kept: Vec<(Vec3, bool)> = seeded
        .iter()
        .filter_map(|&p| inside(&tubes, p, 1e-3).map(|i| (p, i)))
        .collect();
    let probes: Vec<Vec3> = kept.iter().map(|&(p, _)| p).collect();
    let found = contains_points(&at_one, &probes, &tolerance(1.0)).unwrap();
    let inside_count = kept.iter().filter(|(_, i)| *i).count();
    assert!(
        inside_count > kept.len() / 5 && inside_count < kept.len() * 4 / 5,
        "{what}: the points are not spread over both sides ({inside_count} of {})",
        kept.len()
    );
    for ((p, expected), got) in kept.iter().zip(&found) {
        assert_eq!(*got, Some(*expected), "{what}: the point {p:?}");
    }
    // Watertight, and the mesh encloses the volume.
    let chord = 2e-3;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let welded = tessellate_body(&at_one, &params)
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .weld()
        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - measured.volume).abs() <= measured.area * chord,
        "{what}: mesh volume {} against {}",
        welded.volume(),
        measured.volume
    );
    // The same at the other scales, to the measure of scale 1.
    for k in [1e-3, 1e6] {
        let body = unite(groups, k);
        let m = mass(&body);
        let (volume, area) = (m.volume / (k * k * k), m.area / (k * k));
        assert!(
            (volume - measured.volume).abs() <= 1e-9 * measured.volume
                && (area - measured.area).abs() <= 1e-9 * measured.area,
            "{what} at scale {k}: volume {volume}, area {area} against {} and {}",
            measured.volume,
            measured.area
        );
    }
}

/// Five tubes turned about the upright axis through `about`.
fn pattern(about: Vec3) -> Vec<Tube> {
    (0..5)
        .map(|i| {
            let turn = Transform::rotation_about(
                about,
                Vec3::new(0.0, 0.0, 1.0),
                f64::from(i) * TAU / 5.0,
            )
            .unwrap();
            Tube {
                start: turn.point(Vec3::ZERO),
                axis: turn.vector(Vec3::new(0.0, -1.0, 0.0)),
            }
        })
        .collect()
}

#[test]
fn four_patterns_of_five_tubes_unite() {
    let at = Vec3::new(7.5, 2.5, 0.0);
    let groups: Vec<Vec<Tube>> = (0..4)
        .map(|j| pattern(at + Vec3::new(3.0 * f64::from(j), 1.7 * f64::from(j), 0.0)))
        .collect();
    holds(&groups, "four patterns");
}

/// The smallest case of the patterns: a tube, a tube across its end that
/// leaves a hole with a spline edge in its bore, and a tube beside the
/// first, its axis a third of a unit or so off, whose outer wall meets
/// the bore at a few degrees along lines across that hole. Of these 48
/// placements 30 were refused before (as a bug of the regions, as a lost
/// imprint, or as a curve partly on a boundary). Every placement is
/// checked in full at scale 1, every fourth held to all of [`holds`].
#[test]
fn a_tube_beside_a_tube_with_a_hole_unites() {
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    for a in 0..6 {
        for d in 0..8 {
            let groups = vec![
                vec![
                    Tube {
                        start: Vec3::ZERO,
                        axis: x,
                    },
                    Tube {
                        start: Vec3::new(5.0 + 0.7 * f64::from(a), -20.0, 0.0),
                        axis: y,
                    },
                ],
                vec![Tube {
                    start: Vec3::new(0.37 * f64::from(a), 0.3 + 0.04 * f64::from(d), 0.0),
                    axis: x,
                }],
            ];
            let what = format!("placement {a} {d}");
            if (a + d) % 4 == 0 {
                holds(&groups, &what);
            } else {
                unite(&groups, 1.0);
            }
        }
    }
}

/// The reference itself, against closed forms: one tube, two apart, and
/// two on one axis that overlap over a third of their length.
#[test]
fn the_reference_volume_holds_to_closed_forms() {
    let wall = std::f64::consts::PI * (OUTER * OUTER - BORE * BORE) * LENGTH;
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let one = Tube {
        start: Vec3::ZERO,
        axis: x,
    };
    let cases = [
        (vec![one], wall),
        (
            vec![
                one,
                Tube {
                    start: Vec3::new(0.0, 30.0, 0.0),
                    axis: y,
                },
            ],
            2.0 * wall,
        ),
        (
            vec![
                one,
                Tube {
                    start: Vec3::new(10.0, 0.0, 0.0),
                    axis: x,
                },
            ],
            wall * 25.0 / 15.0,
        ),
    ];
    for (tubes, expected) in cases {
        let found = reference_volume(&tubes);
        assert!(
            (found - expected).abs() <= 1e-9 * expected,
            "{found} against {expected}"
        );
    }
}

/// The tube beside the first ever closer to touching it inside: at an
/// offset of 0.3125 the outer wall of the one (radius 10) touches the
/// bore of the other (radius 9.6875), and just past that they cross at a
/// few hundred-thousandths of a radian. Each placement unites, checked in
/// full, or is refused as faces at the edge of tolerance (they go through
/// each other by between half and twice the tolerance: the walls at 0.312501
/// were refused as a bug of the regions before, their crossing taken for a
/// touch), never anything else.
#[test]
fn a_tube_ever_closer_to_touching_inside_unites_or_is_refused_at_the_edge_of_tolerance() {
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let tol = tolerance(1.0);
    let mut refused = 0;
    for x_at in [5.0, 6.4] {
        let first = unite(
            &[vec![
                Tube {
                    start: Vec3::ZERO,
                    axis: x,
                },
                Tube {
                    start: Vec3::new(x_at, -20.0, 0.0),
                    axis: y,
                },
            ]],
            1.0,
        );
        for dy in [
            0.3124999, 0.3125, 0.3125001, 0.3125005, 0.312501, 0.312502, 0.312505, 0.31251,
            0.31255, 0.3126,
        ] {
            let what = format!("x {x_at}, offset {dy}");
            let b = Tube {
                start: Vec3::new(0.0, dy, 0.0),
                axis: x,
            }
            .body(1.0);
            match boolean(&first, &b, BooleanOp::Union, &tol) {
                Ok(r) => {
                    assert_eq!(craft3d_topo::check(&r.body, &tol), [], "{what}");
                    verified(&first, &b, BooleanOp::Union, &r.body, &tol, &what);
                }
                Err(e) => {
                    let said = e.to_string();
                    assert!(
                        said.contains("between half and twice the tolerance")
                            || said.contains("so shallow"),
                        "{what}: {said}"
                    );
                    refused += 1;
                }
            }
        }
    }
    // The walls at 0.312501 and 0.312502 at least.
    assert!(refused >= 2, "{refused} refused");
}
