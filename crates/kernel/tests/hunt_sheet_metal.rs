//! A hunt for wrong sheet-metal parts (docs/design/sheetmetal.md): seeded
//! random parts held to closed forms worked out here, apart from the
//! kernel, at 1e-3, 1 and 1e6, in the world and in frames turned and moved
//! far off:
//!
//! - **flanges** on a rectangle's edges, one to four, at random angles
//!   (tiny, square, past square, up to folded back), up and down, radii
//!   (sharp, a hair, about the thickness, larger), K-factors 0 to 1,
//!   lengths to the tangent line or the mould line, insets, a flange on a
//!   flange's tip (a return or a hem), corner reliefs of every shape, a
//!   hole of a circle or of arcs of one circle in the base: the volume is
//!   `T` times the flats plus `θ w T (R + T/2)` per bend, the flat area
//!   the flats plus `w θ (R + K T)` per bend, the outline's area (Green's
//!   theorem on the kernel's outline, with curves written out here) the
//!   same, each bend's neutral band developed as wide as `θ (R + K T)`,
//!   fold then unfold landing every flat's vertex on the pattern, and the
//!   part's box in the world against the section's closed form;
//! - **jogs** across the base, with flanges on the edges it does not cut:
//!   the web `(h − (2R + T)(1 − cos α)) / sin α`, the volume and the flat
//!   area grown by the web's slab and the two bends';
//! - **curved flanges** on a disc's rim, a collar round a round hole and
//!   the arc of a half-round plate: the volume by Pappus, the flat area by
//!   the annular sectors of the radial development;
//! - every part valid, measurable, its mesh closed and of its volume, not
//!   through itself (a part that passes through itself is counted as
//!   refused: regeneration refuses it), and points classified by rays: in
//!   each flat at mid-thickness inside, past the base's skins outside
//!   where no flange leans over it.
//!
//! The gated sweeps take a minute or two; the wide ones are `#[ignore]`d.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use boolean_common::Rng;
use craft3d_kernel::profile::{Curve2, Profile, Shape as PShape, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::curved::CurvedSpec;
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, FlangeSpec, JogSpec, LengthFrom, On, Part, Relief, ReliefKind,
    SheetFace, Spec, build_with, developed_allowance, flat_area, outline,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::FaceId;
use hunt_sheet_common::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const SCALES: [f64; 4] = [1e-3, 1.0, 1.0, 1e6];

fn line(id: u64, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// One flange as drawn, with what the closed forms need.
#[derive(Clone, Debug)]
struct Fl {
    spec: FlangeSpec,
    /// The width of its bend and flat.
    w: f64,
    /// The flat's length beyond the strip.
    flat: f64,
    /// Its base edge (1 bottom, 2 right, 3 top, 4 left), or none for a
    /// flange on a tip.
    edge: Option<usize>,
}

impl Fl {
    fn ba(&self, t: f64) -> f64 {
        self.spec.angle * (self.spec.radius + self.spec.k * t)
    }
    /// The bend's material: `θ w T (R + T/2)`.
    fn bend_volume(&self, t: f64) -> f64 {
        self.spec.angle * self.w * t * (self.spec.radius + 0.5 * t)
    }
}

/// A part as drawn: its spec, curved flanges, and the closed forms.
struct Drawn {
    what: String,
    spec: Spec,
    curved: Vec<CurvedSpec>,
    volume: f64,
    flat_area: f64,
    /// The box of the part in the plane's axes, when worked out.
    bbox: Option<(Vec3, Vec3)>,
    /// Points of the base, in the pattern's axes, in the material away
    /// from every edge and hole.
    base_points: Vec<P2>,
    /// Whether a flange leans over the base's top (up) or bottom (down).
    over: [bool; 2],
    /// The flanges as drawn.
    flanges: Vec<Fl>,
    /// A hole across a bend: its centre and radius in the pattern.
    across: Option<(P2, f64)>,
}

/// An angle: tiny, square, past square, near folded back or folded back.
fn angle(rng: &mut Rng) -> f64 {
    match rng.below(10) {
        0 => 10f64.ppowf(-rng.between(1.0, 5.0)),
        1 => FRAC_PI_2,
        2 => PI - 10f64.ppowf(-rng.between(1.0, 6.0)),
        3 => PI,
        4 => FRAC_PI_2 + (rng.between(-1.0, 1.0) * 10f64.ppowf(-rng.between(3.0, 9.0))),
        _ => rng.between(1.0, 179.0).to_radians(),
    }
}

fn radius(rng: &mut Rng, t: f64) -> f64 {
    match rng.below(8) {
        0 => 0.0,
        1 => t * (1.0 + rng.between(-1.0, 1.0) * 10f64.ppowf(-rng.between(2.0, 9.0))),
        2 => t * 10f64.ppowf(-rng.between(2.0, 4.0)),
        _ => t * 10f64.ppowf(rng.between(-1.0, 1.0)),
    }
}

fn k_factor(rng: &mut Rng) -> f64 {
    match rng.below(5) {
        0 => 0.0,
        1 => 0.5,
        2 => 1.0,
        3 => 0.33,
        _ => rng.unit(),
    }
}

fn direction(rng: &mut Rng) -> Direction {
    if rng.below(2) == 0 {
        Direction::Up
    } else {
        Direction::Down
    }
}

/// The box of one flange's section about its edge: `u` along the outward
/// direction from the edge, `z` along the plane's normal, for the bend
/// and the flat at their closed forms (a bend down is a bend up turned
/// over the base's middle plane).
fn section_box(f: &Fl, t: f64) -> ([f64; 2], [f64; 2]) {
    let (r, th, lf) = (f.spec.radius, f.spec.angle, f.flat);
    let ro = r + t;
    let (s, c) = th.psin_cos();
    let e0 = [ro * s, t + r - ro * c];
    let e1 = [r * s, t + r - r * c];
    let tip0 = [e0[0] + lf * c, e0[1] + lf * s];
    let tip1 = [e1[0] + lf * c, e1[1] + lf * s];
    let arc_u = ro * th.min(FRAC_PI_2).psin();
    let us = [0.0, arc_u, e0[0], e1[0], tip0[0], tip1[0]];
    let zs = [0.0, t, e0[1], e1[1], tip0[1], tip1[1]];
    let u = [
        us.iter().copied().fold(f64::INFINITY, f64::min),
        us.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ];
    let mut z = [
        zs.iter().copied().fold(f64::INFINITY, f64::min),
        zs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ];
    if f.spec.direction == Direction::Down {
        z = [t - z[1], t - z[0]];
    }
    (u, z)
}

/// A rectangle with flanges (and maybe a tip flange, reliefs, insets, a
/// hole, a jog), drawn at scale `k`.
fn flanged(rng: &mut Rng, k: f64, jog: bool) -> Drawn {
    let w = rng.between(20.0, 80.0) * k;
    let d = rng.between(20.0, 80.0) * k;
    let t = rng.between(0.3, 3.0) * k;
    let c = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    let mut what = vec![format!("base {w:e} x {d:e}, T {t:e}")];
    // A hole in the middle third.
    let mut hole_area = 0.0;
    let mut hole: Option<(P2, f64)> = None;
    if rng.below(3) == 0 {
        let r = rng.between(0.05, 0.15) * w.min(d);
        let hc = [w * rng.between(0.4, 0.6), d * rng.between(0.4, 0.6)];
        if jog {
            // Keep it off the jog's line (at x in 0.3..0.7 of w): in the left fifth.
        }
        let hc = if jog { [0.15 * w, hc[1]] } else { hc };
        let r = if jog { r.min(0.1 * w) } else { r };
        if rng.below(2) == 0 {
            curves.push(SketchCurve {
                id: 9,
                curve: Curve2::Circle {
                    center: hc,
                    radius: r,
                },
            });
            what.push(format!("hole r {r:e}"));
        } else {
            let n = 2 + rng.below(5) as usize;
            let start = rng.between(0.0, TAU);
            let cu = cuts(rng, n, start);
            for (i, s) in split_circle(hc, r, &cu).iter().enumerate() {
                if let Seg::Arc { c, from, to, .. } = s {
                    curves.push(SketchCurve {
                        id: 9 + i as u64,
                        curve: Curve2::Arc {
                            center: *c,
                            start: *from,
                            end: *to,
                        },
                    });
                }
            }
            what.push(format!("hole r {r:e} of {n} arcs"));
        }
        hole_area = PI * r * r;
        hole = Some((hc, r));
    }
    let profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    let base: Profile = profiles.into_iter().next().unwrap();
    // Flanges: on which edges.
    let edges: Vec<usize> = if jog {
        let mut e = Vec::new();
        if rng.below(2) == 0 {
            e.push(2);
        }
        if rng.below(2) == 0 {
            e.push(4);
        }
        e
    } else {
        let mut e: Vec<usize> = (1..=4).filter(|_| rng.below(2) == 0).collect();
        if e.is_empty() {
            e.push(1 + rng.below(4) as usize);
        }
        e
    };
    let edge_len = |e: usize| if e % 2 == 1 { w } else { d };
    let relief = if edges.len() >= 2 && rng.below(3) == 0 {
        let kind = [ReliefKind::Rect, ReliefKind::Round, ReliefKind::Tear][rng.below(3) as usize];
        Some(Relief {
            kind,
            size: rng.between(0.5, 3.0) * t,
        })
    } else {
        None
    };
    let mut flanges: Vec<Fl> = Vec::new();
    let mut insets_used = [[0.0; 2]; 5];
    for &e in &edges {
        let th = angle(rng);
        let r = radius(rng, t);
        let kk = k_factor(rng);
        let l = rng.between(2.0, 30.0) * k;
        let from = if rng.below(4) == 0 {
            LengthFrom::Mould
        } else {
            LengthFrom::Tangent
        };
        let inset = if rng.below(5) == 0 {
            [
                if rng.below(2) == 0 {
                    0.0
                } else {
                    rng.between(0.05, 0.3) * edge_len(e)
                },
                if rng.below(2) == 0 {
                    0.0
                } else {
                    rng.between(0.05, 0.3) * edge_len(e)
                },
            ]
        } else {
            [0.0; 2]
        };
        insets_used[e] = inset;
        let flat = match from {
            LengthFrom::Tangent => l,
            LengthFrom::Mould => l - (r + t) * (0.5 * th).ptan(),
        };
        flanges.push(Fl {
            spec: FlangeSpec {
                on: On::Base(e as u64),
                length: l,
                angle: th,
                direction: direction(rng),
                radius: r,
                k: kk,
                from,
                inset,
                outline: None,
            },
            w: edge_len(e) - inset[0] - inset[1],
            flat,
            edge: Some(e),
        });
    }
    // Corners where two flanges reach the corner: edge e ends at corner e
    // (between e and e % 4 + 1).
    let mut relieved_area = 0.0;
    let mut relieved_corners = Vec::new();
    if let Some(rel) = relief {
        for e in 1..=4usize {
            let next = e % 4 + 1;
            let (Some(a), Some(b)) = (
                flanges.iter().position(|f| f.edge == Some(e)),
                flanges.iter().position(|f| f.edge == Some(next)),
            ) else {
                continue;
            };
            if insets_used[e][1] != 0.0 || insets_used[next][0] != 0.0 {
                continue;
            }
            let r = rel.size;
            relieved_area += match rel.kind {
                ReliefKind::Rect => r * r,
                ReliefKind::Round => 0.25 * PI * r * r,
                ReliefKind::Tear => r * r + 0.75 * PI * r * r,
            };
            flanges[a].w -= r;
            flanges[b].w -= r;
            relieved_corners.push(e);
        }
        what.push(format!(
            "relief {} {:e} at corners {relieved_corners:?}",
            rel.kind.name(),
            rel.size
        ));
    }
    // A flange on a tip: a return or a hem.
    let mut tip_volume = 0.0;
    let mut tip_area = 0.0;
    let mut has_tip = false;
    if !jog && rng.below(4) == 0 {
        let i = rng.below(flanges.len() as u64) as usize;
        let parent_w = flanges[i].w;
        let th = if rng.below(2) == 0 { PI } else { angle(rng) };
        let r = radius(rng, t);
        let kk = k_factor(rng);
        let l = rng.between(2.0, 15.0) * k;
        let f = Fl {
            spec: FlangeSpec {
                on: On::Tip(i),
                length: l,
                angle: th,
                direction: direction(rng),
                radius: r,
                k: kk,
                from: LengthFrom::Tangent,
                inset: [0.0; 2],
                outline: None,
            },
            w: parent_w,
            flat: l,
            edge: None,
        };
        tip_volume = f.bend_volume(t) + f.w * f.flat * t;
        tip_area = f.w * (f.ba(t) + f.flat);
        flanges.push(f);
        has_tip = true;
    }
    for f in &flanges {
        what.push(format!(
            "flange on {:?}: {:.12} deg {:?} R {:e} K {} L {:e} {:?} inset {:?}",
            f.spec.on,
            f.spec.angle.to_degrees(),
            f.spec.direction,
            f.spec.radius,
            f.spec.k,
            f.spec.length,
            f.spec.from,
            f.spec.inset
        ));
    }
    let base_area = w * d - hole_area - relieved_area;
    let straight: Vec<&Fl> = flanges.iter().filter(|f| f.edge.is_some()).collect();
    let mut volume = t * base_area + tip_volume;
    let mut flat = base_area + tip_area;
    for f in &straight {
        volume += f.bend_volume(t) + f.w * f.flat * t;
        flat += f.w * (f.ba(t) + f.flat);
    }
    // A jog across the base, square to x at 0.3..0.7 of the width.
    let mut jogs = Vec::new();
    let mut jogged = false;
    if jog {
        let x = rng.between(0.3, 0.7) * w;
        let alpha = match rng.below(4) {
            0 => FRAC_PI_2,
            1 => 10f64.ppowf(-rng.between(0.5, 3.0)),
            2 => PI - 10f64.ppowf(-rng.between(0.5, 3.0)),
            _ => rng.between(5.0, 175.0).to_radians(),
        };
        let r = radius(rng, t);
        let kk = k_factor(rng);
        let min_h = (2.0 * r + t) * (1.0 - alpha.pcos());
        let h = match rng.below(3) {
            0 => min_h + rng.between(0.0, 3.0) * t,
            1 => min_h * (1.0 + 10f64.ppowf(-rng.between(1.0, 8.0))) + 2e-6 * k,
            _ => rng.between(0.5, 20.0) * t + min_h,
        };
        let dir = direction(rng);
        let ids = [flanges.len() as u64 + 1, flanges.len() as u64 + 2];
        jogs.push(JogSpec {
            line: [[x, -0.1 * d], [x, 1.1 * d]],
            offset: h,
            angle: alpha,
            direction: dir,
            radius: r,
            k: kk,
            ids,
        });
        let web = (h - (2.0 * r + t) * (1.0 - alpha.pcos())) / alpha.psin();
        let ba = alpha * (r + kk * t);
        volume += t * d * web + 2.0 * alpha * d * t * (r + 0.5 * t);
        flat += d * (2.0 * ba + web);
        what.push(format!(
            "jog at x {x:e} by {h:e} at {:.12} deg {dir:?} R {r:e} K {kk} (web {web:e})",
            alpha.to_degrees()
        ));
        jogged = true;
    }
    // Bend reliefs at the ends of bends inside their edges (insets): a
    // notch of `r²` (rect), `π r²/4` (round) or `r² + 3π r²/4` (tear)
    // each.
    let mut bend_relief = None;
    let ends: usize = straight
        .iter()
        .map(|f| f.spec.inset.iter().filter(|i| **i > 0.0).count())
        .sum();
    if ends > 0 && rng.below(2) == 0 {
        let kind = [ReliefKind::Rect, ReliefKind::Round, ReliefKind::Tear][rng.below(3) as usize];
        let r = rng.between(0.3, 1.5) * t;
        let notch = match kind {
            ReliefKind::Rect => r * r,
            ReliefKind::Round => 0.25 * PI * r * r,
            ReliefKind::Tear => r * r + 0.75 * PI * r * r,
        };
        volume -= t * notch * ends as f64;
        flat -= notch * ends as f64;
        bend_relief = Some(Relief { kind, size: r });
        what.push(format!("bend relief {} {r:e} at {ends} ends", kind.name()));
    }
    // A round hole across a bend (a cut-out in the pattern over the
    // bend's strip): it takes `T` per area off the flats and
    // `T (R + T/2) / (R + K T)` per area off the strip.
    let mut cutouts = Vec::new();
    let mut across = None;
    if hole.is_none() && !jogged && rng.below(3) == 0 {
        let pick = flanges
            .iter()
            .position(|f| f.edge.is_some() && f.spec.radius > 0.0 && f.flat > 2.0 * t);
        if let Some(i) = pick {
            let f = &flanges[i];
            let e = f.edge.unwrap();
            let ba = f.ba(t);
            let len = edge_len(e);
            let depth = if e % 2 == 1 { d } else { w };
            let rho = rng.between(0.2, 0.45) * (0.4 * f.w).min(ba + f.flat).min(0.3 * depth);
            let cs = rng.between(-0.6 * rho, ba + 0.6 * rho);
            if cs + rho <= ba + 0.8 * f.flat && cs - rho > -0.5 * depth {
                let (ux, uy) = match e {
                    1 => (0.0, -1.0),
                    2 => (1.0, 0.0),
                    3 => (0.0, 1.0),
                    _ => (-1.0, 0.0),
                };
                let (s0, s1) = (c[e - 1], c[e % 4]);
                let dir = [(s1[0] - s0[0]) / len, (s1[1] - s0[1]) / len];
                let along = f.spec.inset[0] + 0.5 * (len - f.spec.inset[0] - f.spec.inset[1]);
                let centre = [
                    s0[0] + along * dir[0] + cs * ux,
                    s0[1] + along * dir[1] + cs * uy,
                ];
                let g = |x: f64| {
                    let x = x.clamp(-rho, rho);
                    x * (rho * rho - x * x).max(0.0).sqrt() + rho * rho * (x / rho).pasin()
                };
                let band = g(ba - cs) - g(-cs);
                let disc = PI * rho * rho;
                let r = f.spec.radius;
                volume -= t * (disc - band) + band * t * (r + 0.5 * t) / (r + f.spec.k * t);
                flat -= disc;
                let profile = detect_profiles(
                    &[SketchCurve {
                        id: 100,
                        curve: Curve2::Circle {
                            center: centre,
                            radius: rho,
                        },
                    }],
                    &tolerance(k),
                )
                .unwrap()
                .remove(0);
                cutouts.push(Cutout {
                    profile,
                    from: "hole across a bend".into(),
                });
                across = Some((centre, rho));
                what.push(format!(
                    "hole r {rho:e} across flange {i}'s bend, its centre {cs:e} across (strip {ba:e}, band {band:e})"
                ));
            }
        }
    }
    // The box, for parts of base flanges alone.
    let bbox = (!has_tip && !jogged).then(|| {
        let mut lo = Vec3::new(0.0, 0.0, 0.0);
        let mut hi = Vec3::new(w, d, t);
        for f in &straight {
            let (u, z) = section_box(f, t);
            let e = f.edge.unwrap();
            // Edge e's outward direction and its span.
            let (ux, uy) = match e {
                1 => (0.0, -1.0),
                2 => (1.0, 0.0),
                3 => (0.0, 1.0),
                _ => (-1.0, 0.0),
            };
            let (s, en) = (c[e - 1], c[e % 4]);
            for p in [s, en] {
                for uu in u {
                    let q = [p[0] + uu * ux, p[1] + uu * uy];
                    lo = Vec3::new(lo.x.min(q[0]), lo.y.min(q[1]), lo.z.min(z[0]));
                    hi = Vec3::new(hi.x.max(q[0]), hi.y.max(q[1]), hi.z.max(z[1]));
                }
            }
        }
        (lo, hi)
    });
    let over = [
        flanges
            .iter()
            .any(|f| f.spec.direction == Direction::Up && f.spec.angle > FRAC_PI_2 - 1e-6),
        flanges
            .iter()
            .any(|f| f.spec.direction == Direction::Down && f.spec.angle > FRAC_PI_2 - 1e-6),
    ];
    // Points in the base, away from its edges by the largest R + T and
    // the reliefs, and off the hole.
    let margin = flanges
        .iter()
        .map(|f| f.spec.radius + t)
        .fold(0.0, f64::max)
        .max(relief.map_or(0.0, |r| 2.0 * r.size))
        .max(bend_relief.map_or(0.0, |r| 2.0 * r.size))
        .max(0.05 * w.min(d));
    let mut base_points = Vec::new();
    let xmax = if jogged {
        jogs[0].line[0][0] - margin
    } else {
        w - margin
    };
    for _ in 0..12 {
        if xmax <= margin || d - margin <= margin {
            break;
        }
        let p = [rng.between(margin, xmax), rng.between(margin, d - margin)];
        if let Some((hc, r)) = hole.or(across)
            && (p[0] - hc[0]).phypot(p[1] - hc[1]) < 1.1 * r
        {
            continue;
        }
        base_points.push(p);
    }
    Drawn {
        what: what.join("; "),
        spec: Spec {
            base,
            plane: Frame::WORLD,
            thickness: t,
            flanges: flanges.iter().map(|f| f.spec.clone()).collect(),
            relief,
            bend_relief,
            corner: Closing::OPEN,
            corners: Vec::new(),
            cutouts,
            jogs,
            ids: Vec::new(),
        },
        curved: Vec::new(),
        volume,
        flat_area: flat,
        bbox,
        base_points,
        over,
        flanges,
        across,
    }
}

/// The section's `∫ ρ dA` about the arc's axis for a curved flange (the
/// bend's ring sector at `x = r sin φ`, the flat at `x = r sin θ + d cos
/// θ`), as `a A + ε M`.
fn pappus(a: f64, eps: f64, theta: f64, r: f64, t: f64, l: f64) -> f64 {
    let ro = r + t;
    let bend_area = theta * (ro * ro - r * r) / 2.0;
    let bend_mx = (1.0 - theta.pcos()) * (ro.ppowi(3) - r.ppowi(3)) / 3.0;
    let flat_area = l * t;
    let flat_mx = theta.psin() * l * (ro * ro - r * r) / 2.0 + theta.pcos() * t * l * l / 2.0;
    a * (bend_area + flat_area) + eps * (bend_mx + flat_mx)
}

/// A part with a curved flange: a disc's rim (shrink), a collar round a
/// round hole of a rectangle (stretch), or the arc of a half-round plate
/// (shrink, over half a turn).
fn curved(rng: &mut Rng, k: f64) -> Drawn {
    let t = rng.between(0.3, 3.0) * k;
    let th = match rng.below(4) {
        0 => FRAC_PI_2,
        1 => PI,
        _ => rng.between(2.0, 178.0).to_radians(),
    };
    let r = match rng.below(3) {
        0 => t * (1.0 + rng.between(-1.0, 1.0) * 1e-6),
        _ => t * 10f64.ppowf(rng.between(-1.0, 0.7)),
    };
    let kk = k_factor(rng);
    let l = rng.between(2.0, 15.0) * k;
    let dir = direction(rng);
    let which = rng.below(3);
    let (base, base_area, a, eps, sweep, what, pts): (
        Profile,
        f64,
        f64,
        f64,
        f64,
        String,
        Vec<P2>,
    ) = match which {
        0 => {
            let a = rng.between(20.0, 60.0) * k;
            let p = detect_profiles(
                &[SketchCurve {
                    id: 1,
                    curve: Curve2::Circle {
                        center: [0.0, 0.0],
                        radius: a,
                    },
                }],
                &tolerance(k),
            )
            .unwrap()
            .remove(0);
            let pts = (0..8)
                .map(|i| polar([0.0, 0.0], 0.5 * a * f64::from(i) / 8.0, f64::from(i)))
                .collect();
            (p, PI * a * a, a, 1.0, TAU, format!("disc r {a:e}"), pts)
        }
        1 => {
            let (w, h) = (100.0 * k, 80.0 * k);
            let a = rng.between(12.0, 25.0) * k;
            let c = [50.0 * k, 40.0 * k];
            let cc = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
            let mut curves: Vec<SketchCurve> = (0..4)
                .map(|i| line(i as u64 + 1, cc[i], cc[(i + 1) % 4]))
                .collect();
            curves.push(SketchCurve {
                id: 5,
                curve: Curve2::Circle {
                    center: c,
                    radius: a,
                },
            });
            let p = detect_profiles(&curves, &tolerance(k)).unwrap().remove(0);
            let pts = (0..8)
                .map(|i| {
                    [
                        (5.0 + 2.0 * f64::from(i)) * k,
                        (5.0 + 8.0 * f64::from(i)) * k,
                    ]
                })
                .collect();
            (
                p,
                w * h - PI * a * a,
                a,
                -1.0,
                TAU,
                format!("collar in a hole r {a:e}"),
                pts,
            )
        }
        _ => {
            // A plate 2a wide, h high, its top a half circle of radius
            // a about (a, h): curve 4 is the arc.
            let a = rng.between(20.0, 50.0) * k;
            let h = rng.between(10.0, 40.0) * k;
            let curves = vec![
                line(1, [0.0, 0.0], [2.0 * a, 0.0]),
                line(2, [2.0 * a, 0.0], [2.0 * a, h]),
                SketchCurve {
                    id: 4,
                    curve: Curve2::Arc {
                        center: [a, h],
                        start: [2.0 * a, h],
                        end: [0.0, h],
                    },
                },
                line(3, [0.0, h], [0.0, 0.0]),
            ];
            let p = detect_profiles(&curves, &tolerance(k)).unwrap().remove(0);
            let pts = (0..8)
                .map(|i| [a * (0.6 + 0.1 * f64::from(i)), h * 0.5])
                .collect();
            (
                p,
                2.0 * a * h + 0.5 * PI * a * a,
                a,
                1.0,
                PI,
                format!("half-round plate r {a:e} h {h:e}"),
                pts,
            )
        }
    };
    let edge = match which {
        0 => 1,
        1 => 5,
        _ => 4,
    };
    let c = CurvedSpec {
        edge,
        length: l,
        angle: th,
        direction: dir,
        radius: r,
        k: kk,
        from: LengthFrom::Tangent,
        id: 1,
    };
    let ba = th * (r + kk * t);
    let tip = a + eps * (ba + l);
    Drawn {
        what: format!(
            "{what}, T {t:e}, curved flange {:.12} deg {dir:?} R {r:e} K {kk} L {l:e}",
            th.to_degrees()
        ),
        spec: Spec {
            base,
            plane: Frame::WORLD,
            thickness: t,
            flanges: Vec::new(),
            relief: None,
            bend_relief: None,
            corner: Closing::OPEN,
            corners: Vec::new(),
            cutouts: Vec::new(),
            jogs: Vec::new(),
            ids: Vec::new(),
        },
        curved: vec![c],
        volume: base_area * t + sweep * pappus(a, eps, th, r, t, l),
        flat_area: base_area + 0.5 * sweep * (tip * tip - a * a).abs(),
        bbox: None,
        base_points: pts,
        over: [
            dir == Direction::Up && th > FRAC_PI_2 - 1e-6,
            dir == Direction::Down && th > FRAC_PI_2 - 1e-6,
        ],
        flanges: Vec::new(),
        across: None,
    }
}

/// `∮ (x dy − y dx) / 2` of the kernel's outline segments, written out
/// here (lines, arcs, circles; splines by their control polygon's
/// Gauss–Legendre on the clamped B-spline).
fn outline_area(loops: &[Vec<craft3d_kernel::profile::Segment>], o: P2) -> f64 {
    let mut sum = 0.0;
    for lp in loops {
        let mut s = 0.0;
        for seg in lp {
            s += match &seg.shape {
                PShape::Line => Seg::Line(seg.start, seg.end).green(o),
                PShape::Arc { center, radius } => {
                    let a0 = (seg.start[1] - center[1]).patan2(seg.start[0] - center[0]);
                    let a1 = (seg.end[1] - center[1]).patan2(seg.end[0] - center[0]);
                    let mut sweep = a1 - a0;
                    if seg.reversed {
                        while sweep >= 0.0 {
                            sweep -= TAU;
                        }
                    } else {
                        while sweep <= 0.0 {
                            sweep += TAU;
                        }
                    }
                    Seg::Arc {
                        c: *center,
                        r: *radius,
                        a0,
                        sweep,
                        from: seg.start,
                        to: seg.end,
                    }
                    .green(o)
                }
                PShape::Circle { center, radius } => Seg::Circle {
                    c: *center,
                    r: *radius,
                    ccw: !seg.reversed,
                }
                .green(o),
                PShape::Spline(sp) => Seg::Spline {
                    degree: sp.degree,
                    knots: sp.knots.clone(),
                    control: sp.control.clone(),
                    back: seg.reversed,
                }
                .green(o),
            };
        }
        sum += s;
    }
    sum
}

/// Builds `drawn` in the plane `plane` and holds it to its closed forms.
fn run_case(tally: &mut Tally, n: usize, rng: &mut Rng, drawn: &mut Drawn, plane: Frame, k: f64) {
    let tol = tolerance(k);
    drawn.spec.plane = plane;
    let what = format!("{} at k {k} in {plane:?}", drawn.what);
    let built = guarded(|| build_with(&drawn.spec, &drawn.curved, None, &tol));
    let part = match built {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
            return;
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            tally.notes.push(format!("case {n} refused: {what}: {e}"));
            return;
        }
        Ok(Ok(p)) => p,
    };
    let mut found = Found::default();
    hold(&part, drawn, &tol, rng, &mut found);
    let crosses = crosses_itself(&part.solid.body, &tol);
    tally.record(n, &what, &found, crosses);
}

/// Whether the point `(x, z)` of the base's section square to a jog's
/// line lies within `m` of the jog's material past its first bend: its
/// web, its second bend or its far side. `x` is across the line, the
/// base's near side below the line's `x` (the first bend's tangent line),
/// and `z` the height from the bottom skin for a jog that bends down; a
/// jog that bends up is the same seen from the top skin (`z` read as
/// `T − z`). Worked out from the jog's spec alone (docs/design/
/// sheetmetal.md › *Jogs across a flat*): the first bend about the axis
/// `R` below the tangent line, its inside the bottom skin; the web from
/// its end along `(cos α, −sin α)`, `(h − (2R + T)(1 − cos α)) / sin α`
/// long; the second bend about the axis `R` beyond the web's top skin at
/// its end, back by `α`; the far side level from its end on.
fn in_jog(j: &JogSpec, t: f64, x: f64, z: f64, m: f64) -> bool {
    let (sa, ca) = j.angle.psin_cos();
    let (r, xj) = (j.radius, j.line[0][0]);
    let web = (j.offset - (2.0 * r + t) * (1.0 - ca)) / sa;
    let (n, u) = ([sa, ca], [ca, -sa]);
    let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
    // The web: from its inside point at the first bend's end.
    let inside = [xj + r * sa, -r + r * ca];
    let d = [x - inside[0], z - inside[1]];
    let (s, across) = (dot(d, u), dot(d, n));
    if s >= -m && s <= web + m && across >= -m && across <= t + m {
        return true;
    }
    // The second bend, about its axis beyond the web's top skin.
    let axis = [
        inside[0] + (t + r) * n[0] + web * u[0],
        inside[1] + (t + r) * n[1] + web * u[1],
    ];
    let q = [x - axis[0], z - axis[1]];
    let rho = q[0].phypot(q[1]);
    let angle = |a: [f64; 2], b: [f64; 2]| (a[0] * b[1] - a[1] * b[0]).abs().patan2(dot(a, b));
    let swept = angle(q, [-n[0], -n[1]]) + angle(q, [0.0, -1.0]) <= j.angle + 1e-9;
    if rho >= r - m && rho <= r + t + m && swept {
        return true;
    }
    // The far side, level, from the second bend's end on.
    x >= axis[0] - m && z >= axis[1] - r - t - m && z <= axis[1] - r + m
}

fn hold(part: &Part, drawn: &Drawn, tol: &Tolerance, rng: &mut Rng, found: &mut Found) {
    let body = &part.solid.body;
    let t = part.thickness;
    if let Some(m) = sound(body, tol, found) {
        let v = drawn.volume;
        let slack = m.bound
            + 1e-8 * v
            + if any_freeform(body) {
                0.5 * tol.linear * m.area
            } else {
                0.0
            };
        if (m.volume - v).abs() > slack {
            found.problems.push(format!(
                "volume {:.17e} against the closed form {v:.17e} ({:.3e} of it)",
                m.volume,
                m.volume / v - 1.0
            ));
        }
        // Points by rays: the base at mid-thickness, past its skins where
        // no flange leans over; each straight flange's flat at its middle.
        let mut pts = Vec::new();
        let base = &part.flats[0];
        for p in &drawn.base_points {
            let at = |z: f64| {
                let q = base.transform.point(Vec3::new(p[0], p[1], z));
                part.plane.point(q.x, q.y, q.z)
            };
            pts.push((at(0.5 * t), true));
            // A point past a skin is outside unless a jog's web, second
            // bend or far side lies there (a jog bent past square runs
            // back under or over the near side: seeded case 295, a jog of
            // 179.7 degrees down whose web lies about 2R under the base's
            // bottom skin, refused until 2026-11-15 by the plate's trace,
            // then built right with these points inside its web, which the
            // rays and the mesh both said).
            let covered = |dir: Direction| {
                drawn
                    .spec
                    .jogs
                    .iter()
                    .any(|j| j.direction == dir && in_jog(j, t, p[0], -0.1 * t, 0.05 * t))
            };
            if !drawn.over[0] && !covered(Direction::Up) {
                pts.push((at(1.1 * t), false));
            }
            if !drawn.over[1] && !covered(Direction::Down) {
                pts.push((at(-0.1 * t), false));
            }
        }
        for (i, f) in drawn.flanges.iter().enumerate() {
            let Some(flat) = part.flats.get(i + 1) else {
                continue;
            };
            if f.flat < 0.2 * t || f.w < 0.2 * t {
                continue;
            }
            let Some(b) = part.bends.get(i) else { continue };
            // The middle of the flat in the pattern: the far tangent line's
            // middle, half the flat on.
            let (s, e) = b.far_line();
            let m2 = [
                0.5 * (s[0] + e[0]) + 0.5 * f.flat * b.across[0],
                0.5 * (s[1] + e[1]) + 0.5 * f.flat * b.across[1],
            ];
            if let Some((hc, r)) = drawn.across
                && (m2[0] - hc[0]).phypot(m2[1] - hc[1]) < 1.1 * r
            {
                continue;
            }
            let q = flat.transform.point(Vec3::new(m2[0], m2[1], 0.5 * t));
            pts.push((part.plane.point(q.x, q.y, q.z), true));
        }
        if !pts.is_empty() {
            rays(body, &pts, tol, found);
        }
        // The box, in the world's axes when the plane is the world.
        // A freeform face adds its whole control net to the box (Body::bbox),
        // so the box is held only where every face is analytic.
        if any_freeform(body) {
            found.notes.push(format!(
                "FREEFORM: {} freeform faces in a part of analytic surfaces",
                body.faces
                    .iter()
                    .filter(|f| matches!(f.surface, craft3d_geom::Surface::Nurbs(_)))
                    .count()
            ));
        }
        if let Some((lo, hi)) = drawn.bbox
            && part.plane == Frame::WORLD
            && !any_freeform(body)
            && let Some((blo, bhi)) = body.bbox().ok().and_then(|b| b.corners())
        {
            let slack = 10.0 * tol.linear;
            let m3 = |v: Vec3| v.x.abs().max(v.y.abs()).max(v.z.abs());
            let off = m3(blo - lo).max(m3(bhi - hi));
            if off > slack {
                found.problems.push(format!(
                    "box {blo:?}..{bhi:?} against the closed form {lo:?}..{hi:?} (off {off:e})"
                ));
            }
        }
    }
    let _ = rng;
    // The flat pattern: its area, its outline's, the developed bends,
    // fold then unfold.
    let a = flat_area(part);
    let want = drawn.flat_area;
    if (a - want).abs() > 1e-9 * want {
        found.problems.push(format!(
            "flat area {a:.17e} against the closed form {want:.17e} ({:.3e} of it)",
            a / want - 1.0
        ));
    }
    let loops = outline(part, tol);
    let o = part.flats[0].outer.first().map_or([0.0, 0.0], |s| s.start);
    let ao = outline_area(&loops, o).abs();
    if (ao - want).abs() > 1e-9 * want + 1e-12 {
        found.problems.push(format!(
            "the outline's area {ao:.17e} against the closed form {want:.17e} ({:.3e} of it)",
            ao / want - 1.0
        ));
    }
    for (i, b) in part.bends.iter().enumerate() {
        match developed_allowance(b, tol) {
            Ok(dv) => {
                if (dv - b.allowance).abs() > 4.0 * tol.linear {
                    found.problems.push(format!(
                        "bend {i}: developed {dv:e} against the allowance {:e}",
                        b.allowance
                    ));
                }
            }
            Err(e) => found.notes.push(format!("bend {i} not developed: {e}")),
        }
    }
    // Fold then unfold: every folded flat's vertex, carried back, is on
    // the pattern's plane at a vertex of the flat's outline.
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let SheetFace::Flat(i) = role else { continue };
        let flat = &part.flats[*i];
        let Some(back) = flat.transform.inverse() else {
            continue;
        };
        let Some(face) = part.sheet.face(FaceId(f as u32)) else {
            continue;
        };
        for l in &face.loops {
            let Some(lp) = part.sheet.loop_(*l) else {
                continue;
            };
            for c in &lp.coedges {
                let Some((v, _)) = part.sheet.coedge_ends(*c) else {
                    continue;
                };
                let world = part.sheet.vertex(v).unwrap().point;
                let (x, y, z) = part.plane.local(world);
                let p = back.point(Vec3::new(x, y, z));
                let mut on = flat
                    .outer
                    .iter()
                    .chain(flat.holes.iter().flatten())
                    .map(|s| (s.start[0] - p.x).phypot(s.start[1] - p.y))
                    .fold(f64::INFINITY, f64::min);
                // A vertex where a hole across a bend crosses the flat's
                // edge lies on the hole's circle.
                if let Some((hc, r)) = drawn.across {
                    on = on.min(((p.x - hc[0]).phypot(p.y - hc[1]) - r).abs());
                }
                if p.z.abs() > tol.linear || on > tol.linear {
                    found.problems.push(format!(
                        "fold then unfold: flat {i}'s vertex lands {on:e} from the pattern, {:e} off its plane",
                        p.z
                    ));
                    return;
                }
            }
        }
    }
}

fn plane(rng: &mut Rng, k: f64) -> Frame {
    if rng.below(2) == 0 {
        Frame::WORLD
    } else {
        frame(rng, k)
    }
}

fn seeded(range: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in range {
        let mut rng = Rng(case_seed(0x5EE7_3E7A, n));
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let mut drawn = match rng.below(6) {
            0 => flanged(&mut rng, k, true),
            1 => curved(&mut rng, k),
            _ => flanged(&mut rng, k, false),
        };
        let p = plane(&mut rng, k);
        run_case(&mut tally, n, &mut rng, &mut drawn, p, k);
    }
    tally
}

/// Cases of the seeded sweep known wrong, by number, each with what is
/// wrong.
const KNOWN_SEEDED: &[usize] = &[
    // A hole across a bend whose inner radius is about a ten-thousandth
    // of the thickness: the body passes the checker and has its closed-form
    // volume but cannot be measured (525, at 1e-3) or meshed (3150 at 1e6,
    // 3348 at 1): `hunt_sheet_repro.rs` ›
    // `a_hole_across_a_tight_bend_cannot_be_meshed`.
    525, 3150, 3348,
];

#[test]
fn seeded_sheet_metal_parts_are_their_closed_forms() {
    let tally = seeded(0..400);
    tally.hold("seeded sheet metal", 0, KNOWN_SEEDED, 120);
}

/// The wide sweep, by hand: `HUNT_FROM`, `HUNT_TO` (default 400..3400).
#[test]
#[ignore = "a wide sweep, run by hand"]
fn seeded_sheet_metal_wide() {
    let from = std::env::var("HUNT_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let to = std::env::var("HUNT_TO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3400);
    let tally = seeded(from..to);
    tally.hold("seeded sheet metal (wide)", from, KNOWN_SEEDED, 0);
}
