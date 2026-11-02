//! Unfolding: a developable face, or a sheet of them connected across
//! shared edges, laid flat isometrically (lengths, angles and areas
//! kept), with the map between the part and its flat pattern both ways.
//!
//! **One face.** A grid over the face's region in its surface's
//! parameters (the box of its boundary's parameters, unwrapped along each
//! loop on a surface that closes on itself; a band between two loops that
//! go once round is cut open along an iso-line) is laid in the plane point
//! by point, each from two points already laid beside it, at their
//! distances in the plane: the chord of the planar image of the surface
//! curve between them, which an isometry keeps (its speed and geodesic
//! curvature). The map takes a point the same way, from the most regular
//! corner of its cell. The grid is doubled until the coarser layout is
//! within half the tolerance of the finer; that difference is the
//! reported error of the map. A face whose curvature would stretch a flat
//! sheet by more than that share of the tolerance is refused with it.
//!
//! **A sheet.** Faces are laid one by one in a walk across shared edges
//! from the first, each placed by the turn and shift of the plane that
//! best lays its image of the shared edge onto its neighbour's; the images
//! must agree within the tolerance (an edge that bends differently on its
//! two sides, as a chine between two panels or a crease, refuses the
//! unfold, naming the edge). Shared edges the walk does not cross are cuts.
//!
//! Method, sources and oracles: docs/design/developable.md.

use crate::KernelError;
use crate::extrude::{OpResult, fill};
use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_geom::Surface;
use craft3d_geom::refine::interpolate;
use craft3d_math::Portable;
use craft3d_math::quadrature::Rule;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, Coedge, EdgeId, FaceId};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const OPERATION: &str = "unfold";

/// Most faces one unfold lays flat.
pub const MAX_UNFOLD_FACES: usize = 256;
/// Most grid points per face.
const MAX_NODES: usize = 1 << 19;
/// Samples per edge for the region and the shared edges.
const EDGE_SAMPLES: usize = 48;
/// Grid cells along the shorter side of a face to start with.
const START_CELLS: usize = 4;
/// The share of the tolerance a face's stretch ([`FaceCurvature`]) may take
/// to be unfolded, and the share the layout's convergence must reach: the
/// same, so that a face the gate lets through converges.
pub const DEVELOPABLE_SHARE: f64 = 0.5;

fn refused(reason: String) -> KernelError {
    KernelError::Refused {
        operation: OPERATION,
        reason,
    }
}

type P2 = [f64; 2];

fn sub2(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn len2(a: P2) -> f64 {
    a[0].phypot(a[1])
}

/// A turn and shift of the plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion2 {
    pub cos: f64,
    pub sin: f64,
    pub shift: P2,
}

impl Motion2 {
    pub const IDENTITY: Motion2 = Motion2 {
        cos: 1.0,
        sin: 0.0,
        shift: [0.0, 0.0],
    };

    pub fn apply(&self, p: P2) -> P2 {
        [
            self.cos * p[0] - self.sin * p[1] + self.shift[0],
            self.sin * p[0] + self.cos * p[1] + self.shift[1],
        ]
    }

    pub fn inverse_apply(&self, p: P2) -> P2 {
        let q = sub2(p, self.shift);
        [
            self.cos * q[0] + self.sin * q[1],
            -self.sin * q[0] + self.cos * q[1],
        ]
    }

    /// `self` after `first`.
    fn after(&self, first: &Motion2) -> Motion2 {
        Motion2 {
            cos: self.cos * first.cos - self.sin * first.sin,
            sin: self.sin * first.cos + self.cos * first.sin,
            shift: self.apply(first.shift),
        }
    }

    /// The motion that best lays `from` onto `to` (least squares; the
    /// points correspond one to one), with the largest distance left.
    fn fit(from: &[P2], to: &[P2]) -> Option<(Motion2, f64)> {
        let n = from.len().min(to.len());
        if n == 0 {
            return None;
        }
        let mean = |pts: &[P2]| {
            let (sx, sy) = pts
                .iter()
                .take(n)
                .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
            [sx / n as f64, sy / n as f64]
        };
        let (cf, ct) = (mean(from), mean(to));
        let (mut dot, mut cross) = (0.0, 0.0);
        for (a, b) in from.iter().zip(to).take(n) {
            let (a, b) = (sub2(*a, cf), sub2(*b, ct));
            dot += a[0] * b[0] + a[1] * b[1];
            cross += a[0] * b[1] - a[1] * b[0];
        }
        let angle = if dot == 0.0 && cross == 0.0 {
            0.0
        } else {
            cross.patan2(dot)
        };
        let (sin, cos) = angle.psin_cos();
        let turned = Motion2 {
            cos,
            sin,
            shift: [0.0, 0.0],
        }
        .apply(cf);
        let motion = Motion2 {
            cos,
            sin,
            shift: sub2(ct, turned),
        };
        let worst = from
            .iter()
            .zip(to)
            .take(n)
            .map(|(a, b)| len2(sub2(motion.apply(*a), *b)))
            .fold(0.0, f64::max);
        Some((motion, worst))
    }
}

/// The grid of one face laid flat.
#[derive(Clone, Debug, PartialEq)]
struct Layout {
    u: (f64, f64),
    v: (f64, f64),
    nu: usize,
    nv: usize,
    /// `(nu + 1) * (nv + 1)` points, `i * (nv + 1) + j`.
    flat: Vec<P2>,
    /// For a face whose side `v = apex.v` is one point of the part (an
    /// apex): the layout about it ([`Apex`]).
    apex: Option<Apex>,
}

/// A face laid about its apex: the side of its region where `v` is
/// `v` collapses into one point of the part, the apex of a cone (the fans
/// of a developable, 2026-10-28). A developable is a cone about such a
/// point, and laid flat a cone opens at its apex by the length of the
/// curve its unit ruling directions draw on the unit sphere (an isometry
/// keeps the angle between two curves, and at the apex the angle between
/// two rulings, measured on the cone, is that arc). So the iso-line from
/// the apex at each `u` is laid from the apex's image in the direction
/// `θ(u) = ∫ (T × T')·n du`, with `T` the iso-line's unit tangent at the
/// apex (`S_v / |S_v|`, `T' = (S_uv − T (T·S_uv)) / |S_v|`) and `n` the
/// outward normal beside it, as its own planar image (its speed and
/// geodesic curvature, [`planar_image`]): exact up to the quadrature, as
/// the grid's rows from a collapsed row are not (laid from neighbours a
/// small angle apart, the layout converges only at first order towards
/// the apex, and the fans ran out of grid; r1-kayakplates, 2026-10-27).
#[derive(Clone, Debug, PartialEq)]
struct Apex {
    v: f64,
    /// `θ` at each column of the grid, `θ(u0) = 0`.
    theta: Vec<f64>,
}

/// The rate `dθ/du` of the directions at the apex row `v` ([`Apex`]),
/// `None` where the iso-line has no direction there.
fn apex_rate(surface: &Surface, outward: bool, u: f64, v: f64, inward: f64) -> Option<f64> {
    let (_, _, sv) = surface.derivatives(u, v);
    let (_, suv, _) = craft3d_geom::curvature::second_derivatives(surface, u, v);
    let speed = sv.length();
    if !(speed > 0.0 && speed.is_finite()) {
        return None;
    }
    let t = sv * (1.0 / speed);
    let dt = (suv - t * t.dot(suv)) * (1.0 / speed);
    let normal = surface
        .normal(u, v + inward)
        .map(|n| if outward { n } else { -n })?;
    let rate = t.cross(dt).dot(normal);
    rate.is_finite().then_some(rate)
}

/// `∫ dθ` from `a` to `b` along the apex row ([`apex_rate`]), by
/// Simpson's rule doubled until two results agree to 1e-13 (at most 2^12
/// steps).
fn apex_turn(surface: &Surface, outward: bool, a: f64, b: f64, v: f64, inward: f64) -> Option<f64> {
    if a == b {
        return Some(0.0);
    }
    let rate = |u: f64| apex_rate(surface, outward, u, v, inward);
    let simpson = |steps: usize| -> Option<f64> {
        let h = (b - a) / steps as f64;
        let mut sum = rate(a)? + rate(b)?;
        for k in 1..steps {
            let w = if k % 2 == 1 { 4.0 } else { 2.0 };
            sum += w * rate(a + h * k as f64)?;
        }
        Some(sum * h / 3.0)
    };
    let mut steps = 8;
    let mut last = simpson(steps)?;
    while steps < 1 << 12 {
        steps *= 2;
        let next = simpson(steps)?;
        let change = (next - last).abs();
        last = next;
        if change <= 1e-13 * (1.0 + next.abs()) {
            break;
        }
    }
    Some(last)
}

/// The image of `(u, v)` laid about the apex row `apex_v`, its iso-line
/// turned by `theta` ([`Apex`]).
fn about_apex(surface: &Surface, outward: bool, u: f64, v: f64, apex_v: f64, theta: f64) -> P2 {
    if v == apex_v {
        return [0.0, 0.0];
    }
    let local = planar_image(surface, outward, [u, apex_v], [u, v]);
    let (sin, cos) = theta.psin_cos();
    [
        cos * local[0] - sin * local[1],
        sin * local[0] + cos * local[1],
    ]
}

/// A small step from the apex row `apex_v` into the region `v`, where the
/// normal is taken.
fn apex_inward(v: (f64, f64), apex_v: f64) -> f64 {
    let step = 1e-3 * (v.1 - v.0);
    if apex_v == v.0 { step } else { -step }
}

/// Which side `v = v0` or `v = v1` of the region collapses into one point
/// of the part, an apex ([`Apex`]): nine points along it within a
/// millionth of the tolerance of each other (or within the rounding of
/// their coordinates' size, if larger) and the surface's normal lost
/// there; `None` for neither (or both). A side within a millionth of the
/// region of the surface's own bound (the feet of the apex's points, put
/// on the surface) is taken at the bound.
fn apex_side(
    surface: &Surface,
    (u0, u1): (f64, f64),
    (v0, v1): (f64, f64),
    tol: f64,
) -> Option<f64> {
    let (_, (d0, d1)) = surface.domain();
    let near = 1e-6 * (v1 - v0);
    let snapped = |v: f64| {
        if (v - d0).abs() <= near {
            d0
        } else if (v - d1).abs() <= near {
            d1
        } else {
            v
        }
    };
    // Within a millionth of the tolerance, or of the rounding of points
    // that far from the origin (an apex row's control points are one
    // point; their sum rounds by a few ulps of its coordinates).
    let collapsed = |v: f64| {
        let first = surface.eval(u0, v);
        let size = first.x.abs().max(first.y.abs()).max(first.z.abs());
        let within = (1e-6 * tol).max(64.0 * f64::EPSILON * size);
        (0..=8).all(|k| {
            let u = u0 + (u1 - u0) * k as f64 / 8.0;
            surface.eval(u, v).distance(first) <= within && collapsed_at(surface, [u, v], 1e-9)
        })
    };
    let (low, high) = (snapped(v0), snapped(v1));
    match (collapsed(low), collapsed(high)) {
        (true, false) => Some(low),
        (false, true) => Some(high),
        _ => None,
    }
}

impl Layout {
    fn uv(&self, i: usize, j: usize) -> P2 {
        [
            self.u.0 + (self.u.1 - self.u.0) * i as f64 / self.nu as f64,
            self.v.0 + (self.v.1 - self.v.0) * j as f64 / self.nv as f64,
        ]
    }

    fn point(&self, i: usize, j: usize) -> P2 {
        self.flat
            .get(i * (self.nv + 1) + j)
            .copied()
            .unwrap_or([f64::NAN, f64::NAN])
    }

    /// The flat point at `(u, v)`: from the corner of its cell where the
    /// surface is most regular, the planar image of the surface curve
    /// from that corner to `(u, v)`, started at the angle the isometry
    /// gives (the corner's edge in the layout, turned by the angle the
    /// two curves make on the surface).
    fn at(&self, surface: &Surface, outward: bool, u: f64, v: f64) -> P2 {
        if let Some(apex) = &self.apex {
            // From the nearest column's direction.
            let fu = (u - self.u.0) / (self.u.1 - self.u.0) * self.nu as f64;
            let i = (fu.round().max(0.0) as usize).min(self.nu);
            let ui = self.uv(i, 0)[0];
            let inward = apex_inward(self.v, apex.v);
            let theta = apex.theta.get(i).copied().unwrap_or(f64::NAN)
                + apex_turn(surface, outward, ui, u, apex.v, inward).unwrap_or(f64::NAN);
            return about_apex(surface, outward, u, v, apex.v, theta);
        }
        let fu = (u - self.u.0) / (self.u.1 - self.u.0) * self.nu as f64;
        let fv = (v - self.v.0) / (self.v.1 - self.v.0) * self.nv as f64;
        let (i, j) = (
            (fu.floor().max(0.0) as usize).min(self.nu.saturating_sub(1)),
            (fv.floor().max(0.0) as usize).min(self.nv.saturating_sub(1)),
        );
        let mut corners = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
        corners.sort_by(|a, b| {
            let area = |(i, j): (usize, usize)| {
                let [u, v] = self.uv(i, j);
                let (_, du, dv) = surface.derivatives(u, v);
                du.cross(dv).length()
            };
            area(*b).total_cmp(&area(*a))
        });
        let (ci, cj) = corners[0];
        let node = self.uv(ci, cj);
        let target = [u, v];
        if sub2(target, node) == [0.0, 0.0] {
            return self.point(ci, cj);
        }
        // The reference: the corner's neighbour along u (or along v where
        // the u-line has no length there), inside the grid.
        let ri = if ci < self.nu { ci + 1 } else { ci - 1 };
        let rj = if cj < self.nv { cj + 1 } else { cj - 1 };
        let (_, du, dv) = surface.derivatives(node[0], node[1]);
        let (reference, ruv) =
            if du.length() * (self.u.1 - self.u.0) > dv.length() * 1e-9 * (self.v.1 - self.v.0) {
                ((ri, cj), self.uv(ri, cj))
            } else {
                ((ci, rj), self.uv(ci, rj))
            };
        let along_ref = planar_image(surface, outward, node, ruv);
        let flat_ref = sub2(self.point(reference.0, reference.1), self.point(ci, cj));
        let start = flat_ref[1].patan2(flat_ref[0]) - along_ref[1].patan2(along_ref[0]);
        let a = du * (ruv[0] - node[0]) + dv * (ruv[1] - node[1]);
        let b = du * (target[0] - node[0]) + dv * (target[1] - node[1]);
        let n = normal_near(surface, node, target).map(|n| if outward { n } else { -n });
        let turn = match n {
            Some(n) => a.cross(b).dot(n).patan2(a.dot(b)),
            None => 0.0,
        };
        let local = planar_image(surface, outward, node, target);
        let (sin, cos) = (start + turn).psin_cos();
        let base = self.point(ci, cj);
        [
            base[0] + cos * local[0] - sin * local[1],
            base[1] + sin * local[0] + cos * local[1],
        ]
    }
}

/// The surface's normal at `p`, or (at an apex or a pole) a little way
/// towards `q`.
fn normal_near(surface: &Surface, p: P2, q: P2) -> Option<Vec3> {
    surface.normal(p[0], p[1]).or_else(|| {
        let d = sub2(q, p);
        surface.normal(p[0] + 1e-6 * d[0], p[1] + 1e-6 * d[1])
    })
}

/// The image in the plane of the surface curve along the straight line
/// from `p` to `q` in parameters, in the frame where it starts at the
/// origin along +x: its end. The image of a curve under an isometry has
/// the curve's geodesic curvature (do Carmo §4-4), so it is the planar
/// curve with the same speed and the same turning, `dθ/dt = c''·(n × c')
/// / |c'|²`: the angle and the end together by Simpson's rule, step by
/// step from each step's start, middle and end (the angle at the middle
/// by the rule's own interpolant), so both are of the fourth order.
///
/// Span by span (2026-11-02): the line is split where it crosses the
/// surface's knots, as the curve's geodesic curvature jumps there (a
/// cubic's second derivative), and each piece is integrated within its
/// span, the next laid from the end of the one before in the direction
/// its image has turned to. A rule across a knot is of the second order
/// only, and the fitted faces of a developable put their knots where its
/// rulings are, which crowd where the walk slows (towards a fold: the
/// strip beside a fold's fan has spans of 4e-4 of the face at the corner
/// where its rail stands still, a cell of its grid crossing a dozen), so
/// the map between the grid's nodes, an image across a cell, did not
/// settle there, and the strip was refused (r1-kayakplates, 2026-10-27,
/// where it was put down to the corner standing still). The edges of the
/// outline were already developed span by span ([`developed`]).
fn planar_image(surface: &Surface, outward: bool, p: P2, q: P2) -> P2 {
    let d = sub2(q, p);
    let mut cuts: Vec<f64> = vec![0.0];
    if let Surface::Nurbs(s) = surface {
        for (k, knots) in [s.knots_u(), s.knots_v()].into_iter().enumerate() {
            let (Some(&from), Some(&step)) = (p.get(k), d.get(k)) else {
                continue;
            };
            if step == 0.0 || !step.is_finite() || !from.is_finite() {
                continue;
            }
            let (lo, hi) = (from.min(from + step), from.max(from + step));
            let first = knots.partition_point(|x| *x <= lo);
            for x in knots.get(first..).unwrap_or(&[]) {
                if *x >= hi {
                    break;
                }
                cuts.push(((x - from) / step).clamp(0.0, 1.0));
            }
        }
    }
    cuts.push(1.0);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let at = |t: f64| [p[0] + t * d[0], p[1] + t * d[1]];
    let (mut end, mut theta) = ([0.0, 0.0], 0.0_f64);
    for w in cuts.windows(2) {
        let [a, b] = w else { continue };
        if b <= a {
            continue;
        }
        let (piece, turn) = planar_image_piece(surface, outward, at(*a), at(*b));
        let (sin, cos) = theta.psin_cos();
        end = [
            end[0] + cos * piece[0] - sin * piece[1],
            end[1] + sin * piece[0] + cos * piece[1],
        ];
        theta += turn;
    }
    end
}

/// How far inside a piece of [`planar_image`] the rates at its two ends
/// are taken, as a share of the piece: so the rate at a knot is the
/// piece's own span's (a spline is evaluated at a knot in the span after
/// it, and where it is only once differentiable there, its second
/// derivative jumps), and at a corner where the surface's row stands
/// still, where `|c'|` is 0 and the turning has only a limit, that limit.
const KNOT_INSET: f64 = 1e-9;

/// One piece of [`planar_image`], within one span of the surface: its end
/// and the angle its image has turned by there.
fn planar_image_piece(surface: &Surface, outward: bool, p: P2, q: P2) -> (P2, f64) {
    // The steps doubled until two results agree to 1e-12 of the segment's
    // length (a curve whose turning varies quickly, as a fitted face's
    // rows between close rulings, is followed as finely as it needs), and
    // the angle, by which the next piece is laid, to 1e-12.
    // Relative to each segment, as each is laid from the ones before it:
    // the images' errors add up along a face, so a long one (the sea
    // kayak's 5 m panels, 2026-10-27, laid in rows of 400 cells) needs
    // them far below the tolerance over its length; at 1e-10 its layouts
    // stopped converging at 5e-7.
    let mut steps = 8;
    let mut last = planar_image_steps(surface, outward, p, q, steps);
    while steps < MAX_IMAGE_STEPS {
        steps *= 2;
        let next = planar_image_steps(surface, outward, p, q, steps);
        let change = len2(sub2(next.0, last.0));
        let turned = (next.1 - last.1).abs();
        last = next;
        if change <= 1e-12 * len2(next.0) && turned <= 1e-12 {
            break;
        }
    }
    last
}

/// Most Simpson steps along one piece of a curve of the layout.
const MAX_IMAGE_STEPS: usize = 256;

/// The end of a piece's image and the angle it has turned by there, by
/// `steps` steps; the rates at its two ends taken [`KNOT_INSET`] of the
/// way inside them.
fn planar_image_steps(surface: &Surface, outward: bool, p: P2, q: P2, steps: usize) -> (P2, f64) {
    let steps = steps.max(1);
    let d = sub2(q, p);
    let mut last_normal: Option<Vec3> = None;
    // The speed and the turning at `t` along the line.
    let mut rate = |t: f64| -> (f64, f64) {
        let (u, v) = (p[0] + t * d[0], p[1] + t * d[1]);
        let (_, su, sv) = surface.derivatives(u, v);
        let (suu, suv, svv) = craft3d_geom::curvature::second_derivatives(surface, u, v);
        let c1 = su * d[0] + sv * d[1];
        let c2 = suu * (d[0] * d[0]) + suv * (2.0 * d[0] * d[1]) + svv * (d[1] * d[1]);
        let n = surface
            .normal(u, v)
            .map(|n| if outward { n } else { -n })
            .or(last_normal);
        last_normal = n;
        let s2 = c1.length_squared();
        let turn = match n {
            Some(n) if s2 > 0.0 => c2.dot(n.cross(c1)) / s2,
            _ => 0.0,
        };
        (s2.sqrt(), turn)
    };
    let h = 1.0 / steps as f64;
    let (mut theta, mut end) = (0.0_f64, [0.0, 0.0]);
    let mut here = rate(KNOT_INSET);
    for k in 0..steps {
        let t = k as f64 * h;
        let mid = rate(t + 0.5 * h);
        let next = rate(if k + 1 == steps {
            1.0 - KNOT_INSET
        } else {
            t + h
        });
        let (w0, wm, w1) = (here.1, mid.1, next.1);
        let theta_m = theta + h / 24.0 * (5.0 * w0 + 8.0 * wm - w1);
        let theta_1 = theta + h / 6.0 * (w0 + 4.0 * wm + w1);
        let dir = |th: f64, sp: f64| [sp * th.pcos(), sp * th.psin()];
        let (d0, dm, d1) = (
            dir(theta, here.0),
            dir(theta_m, mid.0),
            dir(theta_1, next.0),
        );
        end = [
            end[0] + h / 6.0 * (d0[0] + 4.0 * dm[0] + d1[0]),
            end[1] + h / 6.0 * (d0[1] + 4.0 * dm[1] + d1[1]),
        ];
        theta = theta_1;
        here = next;
    }
    (end, theta)
}

/// One face's place in the pattern.
#[derive(Clone, Debug, PartialEq)]
struct Placed {
    face: FaceId,
    surface: Surface,
    outward: bool,
    layout: Layout,
    motion: Motion2,
    /// The boundary in the surface's parameters, unwrapped, per loop: a
    /// closed polygon for the inside test.
    region: Vec<Vec<P2>>,
    /// Per coedge of the face (in loop order), its samples: parameter on
    /// the edge's curve and the surface parameters there.
    edges: Vec<(EdgeId, Vec<(f64, P2)>)>,
    /// The iso-lines a band is cut along, in the surface's parameters.
    cuts: Vec<[P2; 2]>,
    /// Where the cut meets the loops, on the part.
    cut_points: Vec<Vec3>,
}

impl Placed {
    fn flat(&self, uv: P2) -> P2 {
        self.motion
            .apply(self.layout.at(&self.surface, self.outward, uv[0], uv[1]))
    }

    /// Whether `uv` lies in the face's region (even–odd over its loops).
    fn inside(&self, uv: P2) -> bool {
        let mut inside = false;
        for polygon in &self.region {
            let n = polygon.len();
            for k in 0..n {
                let (Some(a), Some(b)) = (polygon.get(k), polygon.get((k + 1) % n)) else {
                    continue;
                };
                if (a[1] > uv[1]) != (b[1] > uv[1]) {
                    let x = a[0] + (uv[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    if uv[0] < x {
                        inside = !inside;
                    }
                }
            }
        }
        inside
    }

    /// Surface parameters of `p`, unwrapped into the layout's box.
    fn params_of(&self, p: Vec3) -> Result<P2, KernelError> {
        let (u, v) = self.surface.project(p).map_err(KernelError::Geometry)?;
        let wrap = |x: f64, (lo, hi): (f64, f64), period: Option<f64>| match period {
            Some(per) if per > 0.0 => {
                let mid = 0.5 * (lo + hi);
                x + ((mid - x) / per).round() * per
            }
            _ => x,
        };
        Ok([
            wrap(u, self.layout.u, self.surface.period_u()),
            wrap(v, self.layout.v, self.surface.period_v()),
        ])
    }
}

/// What the development changed, and how well the map is known.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Distortion {
    /// The largest relative change of an edge's length.
    pub length: f64,
    /// The relative change of the area.
    pub area: f64,
    /// The estimated error of the map, in model units.
    pub map: f64,
}

/// The image of one edge (or cut) of the part in the pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatCurve {
    /// The edge of the part, or `None` for a cut along an iso-line of a
    /// face that closes on itself.
    pub edge: Option<EdgeId>,
    /// The face it bounds.
    pub face: FaceId,
    pub spline: Spline2,
    /// Its length in the part and in the pattern.
    pub length_3d: f64,
    pub length_flat: f64,
    /// How far the spline was found from the mapped edge.
    pub deviation: f64,
}

/// A development: faces of a body laid flat as one piece.
#[derive(Clone, Debug, PartialEq)]
pub struct Development {
    placed: Vec<Placed>,
    curves: Vec<FlatCurve>,
    error: Distortion,
    profiles: Vec<crate::profile::Profile>,
}

/// The faces `faces` of `body` laid flat as one piece.
pub fn develop(
    body: &Body,
    faces: &[FaceId],
    tolerance: &Tolerance,
) -> Result<Development, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if faces.is_empty() {
        return Err(refused("no faces to unfold".to_string()));
    }
    if faces.len() > MAX_UNFOLD_FACES {
        return Err(refused(format!(
            "{} faces; an unfold lays at most {MAX_UNFOLD_FACES} flat",
            faces.len()
        )));
    }
    let set: BTreeSet<FaceId> = faces.iter().copied().collect();
    if set.len() != faces.len() {
        return Err(refused("a face is given twice".to_string()));
    }
    // Which faces of the set use each edge.
    let mut users: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for &face in faces {
        for coedge in coedges_of(body, face)? {
            users.entry(coedge.edge).or_default().push(face);
        }
    }
    let areas = craft3d_topo::face_areas(body);
    let mut area_3d = 0.0;
    for &face in faces {
        let area = areas
            .get(face.0 as usize)
            .and_then(|a| a.as_ref().ok())
            .copied()
            .ok_or_else(|| refused(format!("face {} cannot be measured", face.0)))?;
        area_3d += area;
    }

    // The walk: from the first face, to neighbours across shared edges,
    // the least face first.
    let mut placed: BTreeMap<FaceId, Placed> = BTreeMap::new();
    let mut map_error: f64 = 0.0;
    let first = *faces
        .first()
        .ok_or_else(|| refused("no faces".to_string()))?;
    let mut queue = VecDeque::from([(first, None::<(FaceId, EdgeId)>)]);
    let mut crossed: BTreeSet<EdgeId> = BTreeSet::new();
    while let Some((face, from)) = queue.pop_front() {
        if placed.contains_key(&face) {
            continue;
        }
        // A band reached across a loop of a band is cut where that band's
        // cut meets the loop, so both open along one line of the part.
        let hint = from.and_then(|(neighbour, edge)| {
            let there = placed.get(&neighbour)?;
            let e = body.edge(edge)?;
            there.cut_points.iter().copied().find(|q| {
                e.curve
                    .project(*q)
                    .is_ok_and(|t| e.curve.eval(t).distance(*q) <= tol)
            })
        });
        let (mut here, error) = lay_face(body, face, tolerance, hint)?;
        map_error = map_error.max(error);
        if let Some((neighbour, edge)) = from {
            let there = placed
                .get(&neighbour)
                .ok_or_else(|| refused("lost a face of the walk".to_string()))?;
            let (mine, theirs) = paired_images(&here, there, edge)?;
            let (motion, worst) = Motion2::fit(&mine, &theirs)
                .ok_or_else(|| refused(format!("edge {} has no points", edge.0)))?;
            map_error = map_error.max(worst);
            if worst > tol {
                return Err(refused(format!(
                    "faces {} and {} do not lie flat as one piece: edge {} bends differently on its two sides, so its two images in the plane are up to {worst} apart (more than the tolerance {tol}); unfold them as separate pieces",
                    neighbour.0, face.0, edge.0
                )));
            }
            here.motion = motion.after(&here.motion);
            crossed.insert(edge);
        }
        let mut next: Vec<(FaceId, EdgeId)> = Vec::new();
        for coedge in coedges_of(body, face)? {
            if let Some(others) = users.get(&coedge.edge) {
                for &other in others {
                    if other != face && !placed.contains_key(&other) {
                        next.push((other, coedge.edge));
                    }
                }
            }
        }
        next.sort();
        for (other, edge) in next {
            queue.push_back((other, Some((face, edge))));
        }
        placed.insert(face, here);
    }
    if let Some(alone) = faces.iter().find(|f| !placed.contains_key(f)) {
        return Err(refused(format!(
            "face {} is not connected to face {} across shared edges; unfold separate pieces separately",
            alone.0, first.0
        )));
    }
    let mut list: Vec<Placed> = Vec::with_capacity(faces.len());
    for face in faces {
        if let Some(p) = placed.remove(face) {
            list.push(p);
        }
    }
    // The pattern's own axes: the first vertex of the first face at the
    // origin, its first edge along +x.
    if let Some(head) = list.first()
        && let Some((_, samples)) = head.edges.first()
        && let (Some(&(_, a)), Some(&(_, b))) = (samples.first(), samples.get(1))
    {
        let (pa, pb) = (head.flat(a), head.flat(b));
        let d = sub2(pb, pa);
        let length = len2(d);
        if length > 0.0 {
            let (cos, sin) = (d[0] / length, -d[1] / length);
            let turn = Motion2 {
                cos,
                sin,
                shift: [0.0, 0.0],
            };
            let turned = turn.apply(pa);
            let normal = Motion2 {
                cos,
                sin,
                shift: [-turned[0], -turned[1]],
            };
            for p in &mut list {
                p.motion = normal.after(&p.motion);
            }
        }
    }
    let curves = outline(body, &list, &crossed, tolerance)?;
    let mut length_change: f64 = 0.0;
    for c in &curves {
        if c.length_3d > tol {
            length_change = length_change.max((c.length_flat - c.length_3d).abs() / c.length_3d);
        }
    }
    for c in &curves {
        map_error = map_error.max(c.deviation);
    }
    // The pattern's region: one profile, its area against the part's.
    let sketched: Vec<SketchCurve> = curves
        .iter()
        .enumerate()
        .map(|(k, c)| SketchCurve {
            id: k as u64 + 1,
            curve: Curve2::Spline(c.spline.clone()),
        })
        .collect();
    let profiles = detect_profiles(&sketched, tolerance)?;
    if profiles.len() != 1 {
        return Err(refused(format!(
            "the pattern is {} regions, not one piece: it overlaps itself",
            profiles.len()
        )));
    }
    let area: f64 = profiles.iter().map(crate::profile::Profile::area).sum();
    let area_change = if area_3d > 0.0 {
        (area - area_3d).abs() / area_3d
    } else {
        0.0
    };
    Ok(Development {
        placed: list,
        curves,
        error: Distortion {
            length: length_change,
            area: area_change,
            map: map_error,
        },
        profiles,
    })
}

fn coedges_of(body: &Body, face: FaceId) -> Result<Vec<Coedge>, KernelError> {
    let f = body
        .face(face)
        .ok_or_else(|| refused(format!("there is no face {}", face.0)))?;
    let mut out = Vec::new();
    for &lp in &f.loops {
        let l = body
            .loop_(lp)
            .ok_or_else(|| refused(format!("face {} has a missing loop", face.0)))?;
        out.extend(l.coedges.iter().copied());
    }
    Ok(out)
}

/// The flat images of the samples of `edge` in two placed faces, paired
/// by the parameter on the edge's curve (both sample it at the same
/// parameters). A band's edge cut on its seam is two pieces: all pieces
/// are read, and the point where it is cut, which has two images, is left
/// out.
fn paired_images(a: &Placed, b: &Placed, edge: EdgeId) -> Result<(Vec<P2>, Vec<P2>), KernelError> {
    let samples = |p: &Placed| -> Vec<(f64, P2)> {
        let mut all: Vec<(f64, P2)> = p
            .edges
            .iter()
            .filter(|(id, _)| *id == edge)
            .flat_map(|(_, s)| s.iter().copied())
            .collect();
        all.sort_by(|x, y| x.0.total_cmp(&y.0));
        let once: Vec<(f64, P2)> = all
            .iter()
            .enumerate()
            .filter(|(k, (t, _))| {
                let same =
                    |j: Option<usize>| j.and_then(|j| all.get(j)).is_some_and(|(u, _)| u == t);
                !same(k.checked_sub(1)) && !same(Some(k + 1))
            })
            .map(|(_, s)| (s.0, p.flat(s.1)))
            .collect();
        once
    };
    let (mine, theirs) = (samples(a), samples(b));
    let span = match (theirs.first(), theirs.last()) {
        (Some(f), Some(l)) => (l.0 - f.0).abs().max(f64::MIN_POSITIVE),
        _ => return Err(refused(format!("edge {} has no points", edge.0))),
    };
    let (mut from, mut to) = (Vec::new(), Vec::new());
    for (t, p) in &mine {
        let at = theirs.partition_point(|(u, _)| u < t);
        let near = [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .filter_map(|j| theirs.get(j))
            .find(|(u, _)| (u - t).abs() <= 1e-9 * span);
        if let Some((_, q)) = near {
            from.push(*p);
            to.push(*q);
        }
    }
    if from.len() < 2 {
        return Err(refused(format!(
            "edge {} is sampled differently by its two faces",
            edge.0
        )));
    }
    Ok((from, to))
}

/// The face's boundary in its surface's parameters: per loop, per
/// coedge, samples unwrapped along the loop.
#[allow(clippy::type_complexity)]
fn boundary(
    body: &Body,
    face: FaceId,
) -> Result<(Surface, bool, Vec<Vec<(EdgeId, Vec<(f64, P2)>)>>), KernelError> {
    let f = body
        .face(face)
        .ok_or_else(|| refused(format!("there is no face {}", face.0)))?;
    let surface = f.surface.clone();
    let periods = [surface.period_u(), surface.period_v()];
    let mut loops = Vec::new();
    let mut previous: Option<P2> = None;
    for &lp in &f.loops {
        let l = body
            .loop_(lp)
            .ok_or_else(|| refused(format!("face {} has a missing loop", face.0)))?;
        let mut pieces = Vec::new();
        for coedge in &l.coedges {
            let e = body
                .edge(coedge.edge)
                .ok_or_else(|| refused(format!("face {} has a missing edge", face.0)))?;
            let (a, b) = e.range;
            let mut samples = Vec::with_capacity(EDGE_SAMPLES + 1);
            for k in 0..=EDGE_SAMPLES {
                let frac = k as f64 / EDGE_SAMPLES as f64;
                let t = if coedge.reversed {
                    b - (b - a) * frac
                } else {
                    a + (b - a) * frac
                };
                let p = e.curve.eval(t);
                // From the sample before; a walk from there that does not
                // settle (beside a fan's apex, where a row collapses and
                // the walk is free along it) falls back on the search
                // over the whole surface, as `project_from` asks.
                let (u, v) = match previous {
                    Some(at) => surface
                        .project_from(p, (at[0], at[1]))
                        .or_else(|_| surface.project(p)),
                    None => surface.project(p),
                }
                .map_err(KernelError::Geometry)?;
                let mut uv = [u, v];
                if let Some(at) = previous {
                    for (k, period) in periods.iter().enumerate() {
                        if let (Some(per), Some(x), Some(&near)) =
                            (period, uv.get_mut(k), at.get(k))
                            && *per > 0.0
                        {
                            *x += ((near - *x) / per).round() * per;
                        }
                    }
                }
                samples.push((t, uv));
                previous = Some(uv);
            }
            pieces.push((coedge.edge, samples));
        }
        // A point where the surface collapses (the apex of a fan, a pole)
        // has no one parameter across: it takes the one of the sample
        // beside it, so the loop runs into the pole and out along the
        // collapsed row.
        let collapsed = |uv: &P2| collapsed_at(&surface, *uv, 1e-9);
        let flat: Vec<(usize, usize)> = pieces
            .iter()
            .enumerate()
            .flat_map(|(i, (_, s))| (0..s.len()).map(move |j| (i, j)))
            .collect();
        let uvs: Vec<P2> = flat
            .iter()
            .filter_map(|&(i, j)| pieces.get(i).and_then(|(_, s)| s.get(j)).map(|x| x.1))
            .collect();
        let degenerate: Vec<bool> = uvs.iter().map(collapsed).collect();
        for (k, &(i, j)) in flat.iter().enumerate() {
            if !degenerate.get(k).copied().unwrap_or(false) {
                continue;
            }
            // The nearest sample along the loop that is not at the pole,
            // within the same edge first.
            let same_edge = |m: usize| flat.get(m).is_some_and(|&(e, _)| e == i);
            let found = (1..flat.len()).find_map(|d| {
                [k.checked_add(d), k.checked_sub(d)]
                    .into_iter()
                    .flatten()
                    .find(|&m| same_edge(m) && !degenerate.get(m).copied().unwrap_or(true))
            });
            // Only along a row that is one point of the part (an apex row,
            // a pole): at a corner where a row stands still but goes on
            // (a cone's apex at a corner of a strip, 2026-11-02) the
            // neighbour's parameter is the neighbour's point, and the
            // corner's own image was laid there instead.
            if let Some(near) = found.and_then(|m| uvs.get(m))
                && let Some(slot) = pieces.get_mut(i).and_then(|(_, s)| s.get_mut(j))
            {
                let here = surface.eval(slot.1[0], slot.1[1]);
                let moved = surface.eval(near[0], slot.1[1]);
                let beside = surface.eval(near[0], near[1]);
                if moved.distance(here) <= 1e-6 * beside.distance(here) {
                    slot.1[0] = near[0];
                }
            }
        }
        loops.push(pieces);
        previous = None;
    }
    Ok((surface, f.reversed, loops))
}

/// A loop that goes once round, shifted to start within `[uc, uc + per)`
/// and cut where it crosses `uc` (or `uc + per`): the edge there becomes
/// two pieces, the samples after it a turn back. Returns the loop and the
/// `v` where it crosses.
#[allow(clippy::type_complexity)]
fn cut_open(
    body: &Body,
    surface: &Surface,
    mut pieces: Vec<(EdgeId, Vec<(f64, P2)>)>,
    uc: f64,
    per: f64,
) -> Result<(Vec<(EdgeId, Vec<(f64, P2)>)>, f64), KernelError> {
    let first_u = pieces
        .first()
        .and_then(|(_, s)| s.first())
        .map(|(_, uv)| uv[0])
        .ok_or_else(|| refused("an empty loop".to_string()))?;
    // The way the loop runs round: it starts within the turn on the side
    // it runs into, so a loop that starts on the cut does not cross it.
    let last_u = pieces
        .last()
        .and_then(|(_, s)| s.last())
        .map(|(_, uv)| uv[0])
        .unwrap_or(first_u);
    let down = last_u < first_u;
    let mut shift = ((uc - first_u) / per).ceil() * per;
    if first_u + shift >= uc + per {
        shift -= per;
    }
    let near_cut = ((first_u + shift) - uc).abs() <= 1e-9 * per;
    if down && near_cut {
        shift += per;
    }
    for (_, samples) in &mut pieces {
        for (_, uv) in samples.iter_mut() {
            uv[0] += shift;
        }
    }
    let mut out = Vec::with_capacity(pieces.len() + 1);
    let mut split_at = None;
    let mut crossing = None;
    let mut back = 0.0;
    for (edge, samples) in pieces {
        let shifted: Vec<(f64, P2)> = samples
            .iter()
            .map(|(t, uv)| (*t, [uv[0] + back, uv[1]]))
            .collect();
        let outside = |u: f64| u < uc || u > uc + per;
        let at = shifted.iter().position(|(_, uv)| outside(uv[0]));
        match (at, crossing) {
            (Some(k), None) if k > 0 => {
                let (Some(&(t0, a)), Some(&(t1, b))) = (shifted.get(k - 1), shifted.get(k)) else {
                    out.push((edge, shifted));
                    continue;
                };
                let bound = if b[0] > uc + per { uc + per } else { uc };
                let e = body
                    .edge(edge)
                    .ok_or_else(|| refused(format!("there is no edge {}", edge.0)))?;
                // Bisection on the curve for the crossing of the bound.
                let (mut lo, mut hi) = (t0, t1);
                let mut uv_at = a;
                for _ in 0..60 {
                    let m = 0.5 * (lo + hi);
                    let (u, v) = surface
                        .project_from(e.curve.eval(m), (uv_at[0], uv_at[1]))
                        .map_err(KernelError::Geometry)?;
                    let u = u + ((uv_at[0] - u) / per).round() * per;
                    if (u - bound).signum() == (a[0] - bound).signum() {
                        lo = m;
                        uv_at = [u, v];
                    } else {
                        hi = m;
                    }
                }
                let t = 0.5 * (lo + hi);
                let v = uv_at[1];
                crossing = Some(v);
                let wrap = if bound > uc { -per } else { per };
                let mut before: Vec<(f64, P2)> = shifted.iter().take(k).copied().collect();
                before.push((t, [bound, v]));
                let mut after = vec![(t, [bound + wrap, v])];
                after.extend(
                    shifted
                        .iter()
                        .skip(k)
                        .map(|(t, uv)| (*t, [uv[0] + wrap, uv[1]])),
                );
                back = wrap;
                split_at = Some(out.len() + 1);
                out.push((edge, before));
                out.push((edge, after));
            }
            _ => out.push((edge, shifted)),
        }
    }
    // A boundary that goes back across the cut is not a band this cut
    // opens: refused rather than a crossing outline.
    let slack = 1e-9 * per;
    if out
        .iter()
        .flat_map(|(_, s)| s.iter())
        .any(|(_, uv)| uv[0] < uc - slack || uv[0] > uc + per + slack)
    {
        return Err(refused(
            "a face's boundary goes back across the line its band is cut open along; unfolding it is not supported".to_string(),
        ));
    }
    // Rotated to run from one side of the cut to the other.
    if let Some(at) = split_at {
        out.rotate_left(at);
    }
    // A loop that starts on the cut crosses it there.
    let v = match crossing {
        Some(v) => v,
        None => pieces_start_v(&out)?,
    };
    Ok((out, v))
}

fn pieces_start_v(out: &[(EdgeId, Vec<(f64, P2)>)]) -> Result<f64, KernelError> {
    out.first()
        .and_then(|(_, s)| s.first())
        .map(|(_, uv)| uv[1])
        .ok_or_else(|| refused("an empty loop".to_string()))
}

/// Whether the surface collapses at `uv` (its normal lost: an apex, a
/// pole): `|S_u × S_v|` within `share` of the larger derivative squared.
fn collapsed_at(surface: &Surface, uv: P2, share: f64) -> bool {
    let (_, du, dv) = surface.derivatives(uv[0], uv[1]);
    let size = du.length().max(dv.length());
    du.cross(dv).length() <= share * size * size
}

/// The distance on the surface between the points at `p` and `q`: the
/// length of the surface curve along the straight line in parameters.
fn on_surface(surface: &Surface, rule: &Rule, p: P2, q: P2) -> f64 {
    let d = sub2(q, p);
    rule.points(0.0, 1.0)
        .map(|(t, w)| {
            let (_, du, dv) = surface.derivatives(p[0] + t * d[0], p[1] + t * d[1]);
            w * (du * d[0] + dv * d[1]).length()
        })
        .sum()
}

/// One face laid flat, with the error of the map.
fn lay_face(
    body: &Body,
    face: FaceId,
    tolerance: &Tolerance,
    cut_at: Option<Vec3>,
) -> Result<(Placed, f64), KernelError> {
    let tol = tolerance.linear;
    let (surface, reversed, loops) = boundary(body, face)?;
    let per_u = surface.period_u();
    // Loops that close in parameters, and loops that go once round.
    let mut closed = Vec::new();
    let mut round = Vec::new();
    for (index, pieces) in loops.iter().enumerate() {
        let (Some(first), Some(last)) = (
            pieces.first().and_then(|(_, s)| s.first()),
            pieces.last().and_then(|(_, s)| s.last()),
        ) else {
            continue;
        };
        let gap = sub2(last.1, first.1);
        let size = 1.0 + first.1[0].abs() + first.1[1].abs();
        // Closed in parameters, or into a pole: the loop's two ends are one
        // point of the part and the region closes along the collapsed row.
        let at = |uv: P2| surface.eval(uv[0], uv[1]);
        let pole = collapsed_at(&surface, first.1, 1e-9)
            && at(first.1).distance(at(last.1)) <= tol
            && gap[1].abs() <= 1e-9 * size;
        if pole || gap[0].abs() + gap[1].abs() <= 1e-9 * size {
            closed.push(index);
        } else if per_u
            .is_some_and(|p| (gap[0].abs() - p).abs() < 1e-6 * p && gap[1].abs() <= 1e-9 * size)
        {
            round.push(index);
        } else {
            return Err(refused(format!(
                "face {} has a boundary that does not close in the surface's parameters (it goes round the {}'s v direction, or ends at a pole); unfolding it is not supported",
                face.0,
                surface.kind()
            )));
        }
    }
    let mut cuts = Vec::new();
    let mut cut_points = Vec::new();
    let mut region: Vec<Vec<P2>> = Vec::new();
    let mut edges: Vec<(EdgeId, Vec<(f64, P2)>)> = Vec::new();
    let mut loops = loops;
    match round.as_slice() {
        [] => {}
        [first, second] => {
            let per = per_u.unwrap_or(std::f64::consts::TAU);
            let start = loops
                .get(*first)
                .and_then(|p| p.first())
                .and_then(|(_, s)| s.first())
                .map(|(_, uv)| *uv)
                .ok_or_else(|| refused(format!("face {} has an empty loop", face.0)))?;
            // Cut where asked (where a neighbouring band was cut), else
            // through the first loop's start.
            let uc = match cut_at {
                Some(q) => surface.project(q).map_err(KernelError::Geometry)?.0,
                None => start[0],
            };
            let mut runs: Vec<Vec<P2>> = Vec::new();
            let mut crossings = Vec::new();
            for index in [*first, *second] {
                let pieces = loops
                    .get(index)
                    .cloned()
                    .ok_or_else(|| refused(format!("face {} has an empty loop", face.0)))?;
                let (pieces, crossing) = cut_open(body, &surface, pieces, uc, per)?;
                runs.push(
                    pieces
                        .iter()
                        .flat_map(|(_, s)| s.iter().map(|(_, uv)| *uv))
                        .collect(),
                );
                crossings.push(crossing);
                if let Some(slot) = loops.get_mut(index) {
                    *slot = pieces;
                }
            }
            // The region: one run across the band, the other back.
            let direction = |run: &Vec<P2>| match (run.first(), run.last()) {
                (Some(a), Some(b)) => b[0] - a[0],
                _ => 0.0,
            };
            let mut polygon = runs.first().cloned().unwrap_or_default();
            if let Some(other) = runs.get(1) {
                let mut other = other.clone();
                if direction(&other) * direction(&polygon) > 0.0 {
                    other.reverse();
                }
                polygon.extend(other);
            }
            region.push(polygon);
            let (v1, v2) = (
                crossings.first().copied().unwrap_or(start[1]),
                crossings.get(1).copied().unwrap_or(start[1]),
            );
            cuts.push([[uc, v1], [uc, v2]]);
            cuts.push([[uc + per, v1], [uc + per, v2]]);
            cut_points.push(surface.eval(uc, v1));
            cut_points.push(surface.eval(uc, v2));
        }
        _ => {
            return Err(refused(format!(
                "face {} goes round its {} with {} boundaries that go round; unfolding is supported for a band between two (a face that reaches an apex or a pole is not)",
                face.0,
                surface.kind(),
                round.len()
            )));
        }
    }
    for index in closed {
        if let Some(pieces) = loops.get(index) {
            region.push(
                pieces
                    .iter()
                    .flat_map(|(_, s)| s.iter().map(|(_, uv)| *uv))
                    .collect(),
            );
        }
    }
    for pieces in loops {
        edges.extend(pieces);
    }
    let (mut u0, mut u1, mut v0, mut v1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for polygon in &region {
        for p in polygon {
            u0 = u0.min(p[0]);
            u1 = u1.max(p[0]);
            v0 = v0.min(p[1]);
            v1 = v1.max(p[1]);
        }
    }
    if region.is_empty() {
        let ((a, b), (c, d)) = surface.domain();
        (u0, u1, v0, v1) = (a, b, c, d);
        region.push(vec![[a, c], [b, c], [b, d], [a, d]]);
    }
    if !(u1 > u0 && v1 > v0 && u0.is_finite() && v1.is_finite()) {
        return Err(refused(format!(
            "face {} has no area in its surface's parameters",
            face.0
        )));
    }
    let checked = curvature_over(&surface, (u0, u1), (v0, v1));
    if checked.stretch > DEVELOPABLE_SHARE * tol {
        let p = checked.at;
        return Err(refused(format!(
            "face {} is not developable: its Gaussian curvature reaches {:e} at ({}, {}, {}), which over the face ({} by {}) would stretch a flat sheet by about {:e} (more than {DEVELOPABLE_SHARE} of the tolerance {tol}); an approximate flatten of a doubly curved face is not offered",
            face.0,
            checked.largest,
            p.x,
            p.y,
            p.z,
            checked.lengths.0,
            checked.lengths.1,
            checked.stretch
        )));
    }
    let (length_u, length_v) = checked.lengths;
    let shorter = length_u.min(length_v).max(f64::MIN_POSITIVE);
    let cells = |length: f64| {
        ((length / shorter * START_CELLS as f64).ceil() as usize).clamp(START_CELLS, 512)
    };
    let (mut nu, mut nv) = (cells(length_u), cells(length_v));
    let outward = !reversed;
    let apex = apex_side(&surface, (u0, u1), (v0, v1), tol);
    // The region reaches the apex row.
    let (v0, v1) = match apex {
        Some(a) if a < v0 => (a, v1),
        Some(a) if a > v1 => (v0, a),
        _ => (v0, v1),
    };
    let mut coarse = lay_grid(&surface, outward, (u0, u1), (v0, v1), nu, nv, apex)?;
    let mut before: Option<f64> = None;
    loop {
        if (2 * nu + 1).saturating_mul(2 * nv + 1) > MAX_NODES {
            // A layout that only halves its difference per doubling (first
            // order, where a developable's converges fourfold) is a face
            // whose parametrisation collapses or stands still somewhere
            // (the fans at the ends of a folded sheet, 2026-10-27): not a
            // coarse grid, and not a face that is not developable.
            return Err(refused(format!(
                "face {} did not reach the tolerance within the largest grid: its layouts still differ by {:e} (tolerance {tol}), falling only about twofold per doubling of the grid, as where the face's parametrisation collapses into a point or stands still; unfolding such a face is not supported (it did not converge)",
                face.0,
                before.unwrap_or(f64::NAN)
            )));
        }
        let fine = lay_grid(&surface, outward, (u0, u1), (v0, v1), 2 * nu, 2 * nv, apex)?;
        // The coarse layout laid onto the fine at the common points, then
        // compared everywhere on the fine grid.
        let mut from = Vec::new();
        let mut to = Vec::new();
        for i in 0..=nu {
            for j in 0..=nv {
                if let (Some(a), Some(b)) = (
                    coarse.flat.get(i * (nv + 1) + j),
                    fine.flat.get(2 * i * (2 * nv + 1) + 2 * j),
                ) {
                    from.push(*a);
                    to.push(*b);
                }
            }
        }
        let (motion, _) = Motion2::fit(&from, &to)
            .ok_or_else(|| refused(format!("face {} has an empty grid", face.0)))?;
        let mut error: f64 = 0.0;
        // At the coarse grid's own nodes alone.
        let mut at_nodes: f64 = 0.0;
        for i in 0..=2 * nu {
            for j in 0..=2 * nv {
                let u = u0 + (u1 - u0) * i as f64 / (2 * nu) as f64;
                let v = v0 + (v1 - v0) * j as f64 / (2 * nv) as f64;
                if let Some(b) = fine.flat.get(i * (2 * nv + 1) + j) {
                    let off = len2(sub2(motion.apply(coarse.at(&surface, outward, u, v)), *b));
                    error = error.max(off);
                    if i % 2 == 0 && j % 2 == 0 {
                        at_nodes = at_nodes.max(off);
                    }
                }
            }
        }
        nu *= 2;
        nv *= 2;
        // On a developable face the layout is exact to its quadrature,
        // so the difference falls at least fourfold per doubling (or is
        // there at once); one that does not halve is the face not being
        // developable within the tolerance, not a coarse grid.
        if error > DEVELOPABLE_SHARE * tol
            && let Some(last) = before
            && error > 0.5 * last
        {
            return Err(refused(format!(
                "face {} does not lie flat within the tolerance: its layouts at two grids still differ by {error:e} (more than {DEVELOPABLE_SHARE} of the tolerance {tol}) and do not converge{}",
                face.0,
                if at_nodes <= 0.25 * error {
                    format!(
                        "; the grids' nodes agree to {at_nodes:e} and the map between them does not (where the face's parametrisation stands still or its knots crowd)"
                    )
                } else {
                    String::new()
                }
            )));
        }
        before = Some(error);
        if error <= DEVELOPABLE_SHARE * tol {
            return Ok((
                Placed {
                    face,
                    surface,
                    outward,
                    layout: fine,
                    motion: Motion2::IDENTITY,
                    region,
                    edges,
                    cuts,
                    cut_points,
                },
                error,
            ));
        }
        coarse = fine;
    }
}

/// How far a face is from developable: the largest |K| found on a grid
/// over its region of its surface, where, and the stretch a flat sheet
/// would need over the face.
///
/// The stretch is that of laying the face out from a line through its
/// middle (Fermi coordinates about it: the geodesics square to the line,
/// along which distances are kept). There the metric is `dy² + G ds²`
/// with `∂²(√G)/∂y² = −K √G` (the Jacobi equation, do Carmo §4-6), and
/// the line developed flat with its own geodesic curvature has the same
/// `√G` but for `K = 0`; so a curve at the offset `y` changes its length,
/// to first order in `K`, by `∫∫ (y − η) K dη ds`, at most
/// `∫ w² max|K| / 8 ds` along the line for a face of width `w` across
/// it. Laid from the middle, each half adds its own: the stretch is the
/// larger half's, with the width and the largest |K| across taken at
/// every sample along (so a face whose curvature is found at one end is
/// not charged for its whole length), and of the two directions of the
/// face's parameters the one that gives less (each is an estimate of the
/// same stretch). A face of constant `K` and sides `w ≤ L` gets
/// `|K| w² L / 16`: about the `|K| r³ / 6` of a geodesic disc
/// (Bertrand–Diguet–Puiseux) for a square, far less for a long narrow
/// strip, whose stretch does not grow with the cube of its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCurvature {
    pub largest: f64,
    pub at: Vec3,
    /// Half the diagonal of the face's region (along its parameters).
    pub radius: f64,
    pub stretch: f64,
    /// Whether the curvature could be taken at every point of the grid.
    pub everywhere: bool,
    lengths: (f64, f64),
}

/// The share of the square of its larger tangent below which the area
/// element of a sample of the curvature counts as nearly standing still,
/// and of the face's parameters by which such a sample is moved inward
/// ([`curvature_over`]).
const NEARLY_STILL: f64 = 1e-3;

fn curvature_over(surface: &Surface, (u0, u1): (f64, f64), (v0, v1): (f64, f64)) -> FaceCurvature {
    const N: usize = 32;
    let rule = Rule::new(4);
    let mid = [0.5 * (u0 + u1), 0.5 * (v0 + v1)];
    let length_u = on_surface(surface, &rule, [u0, mid[1]], [u1, mid[1]]);
    let length_v = on_surface(surface, &rule, [mid[0], v0], [mid[0], v1]);
    let radius = 0.5 * length_u.phypot(length_v);
    let mut largest: f64 = 0.0;
    let mut at = mid;
    let mut everywhere = true;
    let sorted = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples.dedup();
        samples
    };
    let (us, vs) = (
        sorted(curvature_samples(surface, true, (u0, u1), N)),
        sorted(curvature_samples(surface, false, (v0, v1), N)),
    );
    // The largest |K| across the face at each sample along it, both ways.
    let mut across_u = vec![0.0_f64; us.len()];
    let mut across_v = vec![0.0_f64; vs.len()];
    for (&u, ku) in us.iter().zip(across_u.iter_mut()) {
        for (&v, kv) in vs.iter().zip(across_v.iter_mut()) {
            // Where the parametrisation nearly stands still (a corner where
            // one row has a turning point: the strip beside the fan at a
            // fold, 2026-10-27) `K = (LN − M²)/(EG − F²)` divides what the
            // fit leaves of the numerator by a denominator that is all but
            // zero; `K` is continuous on the surface, so it is taken a
            // thousandth of the face's parameters inward, where the
            // parametrisation is regular.
            if collapsed_at(surface, [u, v], 1e-6) {
                continue;
            }
            let uv = if collapsed_at(surface, [u, v], NEARLY_STILL) {
                let inward = |x: f64, lo: f64, hi: f64| {
                    let step = (hi - lo) * NEARLY_STILL;
                    if x < 0.5 * (lo + hi) {
                        x + step
                    } else {
                        x - step
                    }
                };
                [inward(u, u0, u1), inward(v, v0, v1)]
            } else {
                [u, v]
            };
            // Where the surface collapses (a fan's apex) the curvature is
            // the rounding of 0 / 0: not a curvature of the face.
            if collapsed_at(surface, uv, 1e-6) {
                continue;
            }
            match craft3d_geom::curvature::gaussian_curvature(surface, uv[0], uv[1]) {
                Some(k) => {
                    let k = k.abs();
                    *ku = ku.max(k);
                    *kv = kv.max(k);
                    if k > largest {
                        largest = k;
                        at = uv;
                    }
                }
                None => everywhere = false,
            }
        }
    }
    let along_u = strip_stretch(surface, &rule, &us, &across_u, true, (v0, v1));
    let along_v = strip_stretch(surface, &rule, &vs, &across_v, false, (u0, u1));
    let stretch = along_u.min(along_v);
    FaceCurvature {
        largest,
        at: surface.eval(at[0], at[1]),
        radius,
        stretch: if stretch.is_nan() {
            f64::INFINITY
        } else {
            stretch
        },
        everywhere,
        lengths: (length_u, length_v),
    }
}

/// The stretch of laying a face out from its middle line along one
/// direction of its parameters ([`FaceCurvature`]): `samples` along that
/// direction (sorted), `largest` the largest |K| across the face at each,
/// `across` the face's range in the other parameter. Per interval between
/// samples, its length along the middle line times the larger of
/// `w² |K| / 8` at its two ends (`w` the face's width across there); the
/// larger of the two halves' sums.
fn strip_stretch(
    surface: &Surface,
    rule: &Rule,
    samples: &[f64],
    largest: &[f64],
    along_u: bool,
    (lo, hi): (f64, f64),
) -> f64 {
    let middle_across = 0.5 * (lo + hi);
    let point = |along: f64, across: f64| -> P2 {
        if along_u {
            [along, across]
        } else {
            [across, along]
        }
    };
    let rate: Vec<f64> = samples
        .iter()
        .zip(largest)
        .map(|(&t, &k)| {
            let w = on_surface(surface, rule, point(t, lo), point(t, hi));
            w * w * k / 8.0
        })
        .collect();
    let (Some(&first), Some(&last)) = (samples.first(), samples.last()) else {
        return 0.0;
    };
    let middle = 0.5 * (first + last);
    let (mut before, mut after) = (0.0_f64, 0.0_f64);
    for (t, r) in samples.windows(2).zip(rate.windows(2)) {
        let ([t0, t1], [r0, r1]) = (t, r) else {
            continue;
        };
        let ds = on_surface(
            surface,
            rule,
            point(*t0, middle_across),
            point(*t1, middle_across),
        );
        let part = r0.max(*r1) * ds;
        if 0.5 * (t0 + t1) < middle {
            before += part;
        } else {
            after += part;
        }
    }
    before.max(after)
}

/// Most samples of the curvature along one direction of a face.
const MAX_CURVATURE_SAMPLES: usize = 1024;

/// Where to sample a face's curvature along one direction over `range`:
/// `n` even steps, and on a spline also the middle of every knot span in
/// the range (a fitted face can curve between rulings, which an even grid
/// coarser than their spacing would not see).
pub fn curvature_samples(
    surface: &Surface,
    along_u: bool,
    (lo, hi): (f64, f64),
    n: usize,
) -> Vec<f64> {
    let mut out: Vec<f64> = (0..=n)
        .map(|k| lo + (hi - lo) * k as f64 / n as f64)
        .collect();
    let knots: Option<&[f64]> = match surface {
        Surface::Nurbs(s) => Some(if along_u { s.knots_u() } else { s.knots_v() }),
        _ => None,
    };
    if let Some(knots) = knots {
        for w in knots.windows(2) {
            if let [a, b] = w
                && b > a
            {
                let m = 0.5 * (a + b);
                if m > lo && m < hi && out.len() < MAX_CURVATURE_SAMPLES {
                    out.push(m);
                }
            }
        }
    }
    out
}

/// [`FaceCurvature`] of face `face` of `body`, over the box of its
/// boundary's parameters (unwrapped along each loop; a face without loops
/// over its surface's domain).
pub fn face_curvature(body: &Body, face: FaceId) -> Result<FaceCurvature, KernelError> {
    let (surface, _, loops) = boundary(body, face)?;
    let (mut u0, mut u1, mut v0, mut v1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for (_, samples) in loops.iter().flatten() {
        for (_, uv) in samples {
            u0 = u0.min(uv[0]);
            u1 = u1.max(uv[0]);
            v0 = v0.min(uv[1]);
            v1 = v1.max(uv[1]);
        }
    }
    if loops.is_empty() {
        ((u0, u1), (v0, v1)) = surface.domain();
    }
    if !(u1 > u0 && v1 > v0 && u0.is_finite() && u1.is_finite() && v0.is_finite() && v1.is_finite())
    {
        return Err(refused(format!(
            "face {} has no area in its surface's parameters",
            face.0
        )));
    }
    Ok(curvature_over(&surface, (u0, u1), (v0, v1)))
}

/// The grid laid flat: rows 0 and 1 as a zig-zag strip, then each later
/// row from the one below, each point from two points laid beside it.
fn lay_grid(
    surface: &Surface,
    outward: bool,
    (u0, u1): (f64, f64),
    (v0, v1): (f64, f64),
    nu: usize,
    nv: usize,
    apex: Option<f64>,
) -> Result<Layout, KernelError> {
    if let Some(apex_v) = apex {
        return lay_about_apex(surface, outward, (u0, u1), (v0, v1), nu, nv, apex_v);
    }
    let width = nv + 1;
    let uv = |i: usize, j: usize| {
        [
            u0 + (u1 - u0) * i as f64 / nu as f64,
            v0 + (v1 - v0) * j as f64 / nv as f64,
        ]
    };
    let mut flat: Vec<Option<P2>> = vec![None; (nu + 1) * width];
    let mut order = Vec::with_capacity(flat.len());
    for i in 0..=nu {
        order.push((i, 0));
        order.push((i, 1));
    }
    for j in 2..=nv {
        for i in 0..=nu {
            order.push((i, j));
        }
    }
    // The size of the face (its corners' distances), for what counts as
    // one point: independent of where the face lies.
    let scale = {
        let (p, q) = (surface.eval(u0, v0), surface.eval(u1, v1));
        let (r, t) = (surface.eval(u1, v0), surface.eval(u0, v1));
        p.distance(q).max(r.distance(t)).max(f64::MIN_POSITIVE)
    };
    let mut framed = false;
    for (i, j) in order {
        let here = uv(i, j);
        let x3 = surface.eval(here[0], here[1]);
        // Neighbours already laid, with their distances on the surface.
        let mut near: Vec<(P2, Vec3, f64)> = Vec::new();
        for (di, dj) in [(-1, 0), (-1, -1), (0, -1), (1, -1), (-1, 1)] {
            let (ni, nj) = (i as isize + di, j as isize + dj);
            if ni < 0 || nj < 0 || ni > nu as isize || nj > nv as isize {
                continue;
            }
            let (ni, nj) = (ni as usize, nj as usize);
            let Some(Some(p)) = flat.get(ni * width + nj) else {
                continue;
            };
            let there = uv(ni, nj);
            let d = len2(planar_image(surface, outward, there, here));
            near.push((*p, surface.eval(there[0], there[1]), d));
        }
        let placed = if near.is_empty() {
            [0.0, 0.0]
        } else if let Some(same) = near.iter().find(|(_, _, d)| *d <= 1e-13 * scale) {
            same.0
        } else if !framed {
            framed = true;
            let (p, _, d) = near
                .first()
                .copied()
                .ok_or_else(|| refused("lost a point".to_string()))?;
            [p[0] + d, p[1]]
        } else {
            // The best-conditioned pair of distinct neighbours.
            let normal = surface.normal(here[0], here[1]);
            let mut best: Option<(f64, P2)> = None;
            for (a, pa) in near.iter().enumerate() {
                for pb in near.iter().skip(a + 1) {
                    let base = pb.1 - pa.1;
                    if base.length() <= 1e-13 * scale || len2(sub2(pb.0, pa.0)) <= 1e-13 * scale {
                        continue;
                    }
                    let (ea, eb) = (x3 - pa.1, x3 - pb.1);
                    let sine =
                        ea.cross(eb).length() / (ea.length() * eb.length()).max(f64::MIN_POSITIVE);
                    if best.as_ref().is_some_and(|(s, _)| *s >= sine) {
                        continue;
                    }
                    let side = match normal {
                        Some(n) => {
                            let s = base.cross(x3 - pa.1).dot(n);
                            if outward == (s >= 0.0) { 1.0 } else { -1.0 }
                        }
                        None => 1.0,
                    };
                    if let Some(p) = circles(pa.0, pa.2, pb.0, pb.2, side) {
                        best = Some((sine, p));
                    }
                }
            }
            match best {
                Some((_, p)) => p,
                None => {
                    return Err(refused(format!(
                        "the grid of a {} cannot be laid flat near ({}, {}, {}): its points there collapse (an apex or a pole in the face)",
                        surface.kind(),
                        x3.x,
                        x3.y,
                        x3.z
                    )));
                }
            }
        };
        if let Some(slot) = flat.get_mut(i * width + j) {
            *slot = Some(placed);
        }
    }
    Ok(Layout {
        u: (u0, u1),
        v: (v0, v1),
        nu,
        nv,
        flat: flat
            .into_iter()
            .map(|p| p.unwrap_or([f64::NAN, f64::NAN]))
            .collect(),
        apex: None,
    })
}

/// The grid of a face laid about its apex row `apex_v` ([`Apex`]).
fn lay_about_apex(
    surface: &Surface,
    outward: bool,
    (u0, u1): (f64, f64),
    (v0, v1): (f64, f64),
    nu: usize,
    nv: usize,
    apex_v: f64,
) -> Result<Layout, KernelError> {
    let inward = apex_inward((v0, v1), apex_v);
    let column = |i: usize| u0 + (u1 - u0) * i as f64 / nu as f64;
    let mut theta = Vec::with_capacity(nu + 1);
    let mut angle = 0.0;
    theta.push(angle);
    for i in 1..=nu {
        angle += apex_turn(surface, outward, column(i - 1), column(i), apex_v, inward)
            .ok_or_else(|| {
                let p = surface.eval(column(i), apex_v);
                refused(format!(
                    "the face cannot be laid about its apex at ({}, {}, {}): the curves from it have no direction there",
                    p.x, p.y, p.z
                ))
            })?;
        theta.push(angle);
    }
    let mut flat = Vec::with_capacity((nu + 1) * (nv + 1));
    for (i, th) in theta.iter().enumerate() {
        for j in 0..=nv {
            let v = v0 + (v1 - v0) * j as f64 / nv as f64;
            let v = if j == nv { v1 } else { v };
            flat.push(about_apex(surface, outward, column(i), v, apex_v, *th));
        }
    }
    Ok(Layout {
        u: (u0, u1),
        v: (v0, v1),
        nu,
        nv,
        flat,
        apex: Some(Apex { v: apex_v, theta }),
    })
}

/// The point at `ra` from `a` and `rb` from `b`, on the left of `a → b`
/// for `side` 1 and on the right for −1.
fn circles(a: P2, ra: f64, b: P2, rb: f64, side: f64) -> Option<P2> {
    let ab = sub2(b, a);
    let d = len2(ab);
    if d.is_nan() || d <= 0.0 {
        return None;
    }
    let along = (ra * ra - rb * rb + d * d) / (2.0 * d);
    let h2 = ra * ra - along * along;
    // The distances of a surface that is not quite flat at this grid do
    // not quite close; a shortfall within rounding of the sides is none.
    let h = if h2 >= 0.0 {
        h2.sqrt()
    } else if -h2 <= 1e-6 * ra * ra {
        0.0
    } else {
        return None;
    };
    let (ex, ey) = (ab[0] / d, ab[1] / d);
    Some([
        a[0] + along * ex - side * h * ey,
        a[1] + along * ey + side * h * ex,
    ])
}

/// The flat images of the boundary of the pattern: every edge the walk
/// did not cross (an edge only one face uses, a seam of a face, an edge
/// shared by faces the walk reached otherwise), and the cuts of the bands,
/// as splines refined until they are within half the tolerance of the
/// mapped edge at the middles between their points (refused when they do
/// not get there).
fn outline(
    body: &Body,
    placed: &[Placed],
    crossed: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> Result<Vec<FlatCurve>, KernelError> {
    let tol = tolerance.linear;
    let rule = Rule::new(6);
    let mut out = Vec::new();
    for p in placed {
        for (edge, samples) in &p.edges {
            if crossed.contains(edge) {
                continue;
            }
            let e = body
                .edge(*edge)
                .ok_or_else(|| refused(format!("there is no edge {}", edge.0)))?;
            // The stretch of the edge these samples cover (a band's edge
            // cut on its seam is two stretches).
            let (lo, hi) = samples
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (t, _)| {
                    (lo.min(*t), hi.max(*t))
                });
            let length_3d = edge_length(&e.curve, lo, hi);
            let (r0, r1) = e.range;
            let along = |t: f64,
                         uv: P2,
                         span: (f64, f64)|
             -> Result<(Vec3, Vec3, Option<Vec3>), KernelError> {
                let (q, c1) = e.curve.derivatives(t);
                // The second derivative by differences of the first,
                // within the span between the knots the parameter is in:
                // central inside it, one-sided of the second order at its
                // ends (the curve's second derivative jumps at a knot).
                let (lo, hi) = (span.0.max(r0), span.1.min(r1));
                let h = (1e-5 * (r1 - r0)).min(0.25 * (hi - lo));
                let d1 = |x: f64| e.curve.derivatives(x).1;
                let c2 = if h.is_nan() || h <= 0.0 {
                    Vec3::ZERO
                } else if t - h >= lo && t + h <= hi {
                    (d1(t + h) - d1(t - h)) * (0.5 / h)
                } else if t + 2.0 * h <= hi {
                    (d1(t + h) * 4.0 - d1(t + 2.0 * h) - c1 * 3.0) * (0.5 / h)
                } else {
                    (c1 * 3.0 - d1(t - h) * 4.0 + d1(t - 2.0 * h)) * (0.5 / h)
                };
                let (u, v) = p
                    .surface
                    .project_from(q, (uv[0], uv[1]))
                    .or_else(|_| p.surface.project(q))
                    .map_err(KernelError::Geometry)?;
                let n = p
                    .surface
                    .normal(u, v)
                    .map(|n| if p.outward { n } else { -n });
                Ok((c1, c2, n))
            };
            let breaks: Vec<f64> = match &e.curve {
                craft3d_geom::Curve3::Nurbs(n) => n.knots().to_vec(),
                _ => Vec::new(),
            };
            if let Some(curve) =
                developed(p, samples, &along, &breaks, Some(*edge), length_3d, tol)?
            {
                out.push(curve);
            }
        }
        for cut in &p.cuts {
            let [a, b] = *cut;
            let n = 32;
            let samples: Vec<(f64, P2)> = (0..=n)
                .map(|k| {
                    let t = k as f64 / n as f64;
                    (t, [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])])
                })
                .collect();
            let d = sub2(b, a);
            let along =
                |t: f64, _: P2, _: (f64, f64)| -> Result<(Vec3, Vec3, Option<Vec3>), KernelError> {
                    let (u, v) = (a[0] + t * d[0], a[1] + t * d[1]);
                    let (_, su, sv) = p.surface.derivatives(u, v);
                    let (suu, suv, svv) =
                        craft3d_geom::curvature::second_derivatives(&p.surface, u, v);
                    let c1 = su * d[0] + sv * d[1];
                    let c2 = suu * (d[0] * d[0]) + suv * (2.0 * d[0] * d[1]) + svv * (d[1] * d[1]);
                    let n = p
                        .surface
                        .normal(u, v)
                        .map(|n| if p.outward { n } else { -n });
                    Ok((c1, c2, n))
                };
            let length_3d = rule.integrate(0.0, 1.0, 64, |t| {
                along(t, a, (0.0, 1.0)).map_or(0.0, |(c1, _, _)| c1.length())
            });
            if let Some(curve) = developed(p, &samples, &along, &[], None, length_3d, tol)? {
                out.push(curve);
            }
        }
    }
    Ok(out)
}

/// The length of a curve between `lo` and `hi`: Gauss–Legendre over
/// every piece between the knots of a spline (its speed has kinks there,
/// which a rule across them does not integrate to the rounding: a
/// reparametrized arc measured 1e-5 short across them), sixteen panels
/// each.
fn edge_length(curve: &craft3d_geom::Curve3, lo: f64, hi: f64) -> f64 {
    let mut breaks = vec![lo];
    if let craft3d_geom::Curve3::Nurbs(n) = curve {
        let mut last = lo;
        for &k in n.knots() {
            if k > last && k < hi {
                breaks.push(k);
                last = k;
            }
        }
    }
    breaks.push(hi);
    let rule = Rule::new(6);
    breaks
        .windows(2)
        .map(|w| match w {
            [a, b] if b > a => rule.integrate(*a, *b, 16, |t| curve.derivatives(t).1.length()),
            _ => 0.0,
        })
        .sum()
}

/// What a curve of the face is at a parameter: its first and second
/// derivatives in space and the face's outward normal there (`None` at a
/// pole). The second derivative is taken within the span given (the
/// stretch between two knots the parameter is in), not across it.
type CurveAt<'a> =
    dyn Fn(f64, P2, (f64, f64)) -> Result<(Vec3, Vec3, Option<Vec3>), KernelError> + 'a;

/// Most Simpson steps per interval between an edge's samples.
const MAX_SUB: usize = 256;

/// The image of a curve of the face in the pattern, developed along the
/// curve itself: the planar curve with the curve's speed and its geodesic
/// curvature (`dθ/dt = c''·(n × c') / |c'|²`, do Carmo §4-4), by
/// Simpson's rule over `sub` steps per interval between `samples`, then
/// laid by the turn and shift that best fits it onto the map's images of
/// the samples. Its length is the curve's by construction, so the map's
/// pointwise error does not add up along it; how far it lies from the
/// map's images, from the spline drawn through it, and how far the
/// spline's length is from the curve's are measured, and the outline is
/// refused when any is beyond what the tolerance allows. `sub` starts at
/// 8 and is doubled, up to [`MAX_SUB`], while the image is farther than
/// half the tolerance from the map's or from its spline.
fn developed(
    p: &Placed,
    samples: &[(f64, P2)],
    at: &CurveAt<'_>,
    breaks: &[f64],
    edge: Option<EdgeId>,
    length_3d: f64,
    tol: f64,
) -> Result<Option<FlatCurve>, KernelError> {
    let what = || {
        format!(
            "face {}{}",
            p.face.0,
            edge.map(|e| format!(", edge {}", e.0)).unwrap_or_default()
        )
    };
    let mut sub = 8;
    loop {
        let mut points: Vec<P2> = Vec::new();
        let mut mids: Vec<P2> = Vec::new();
        let mut at_samples: Vec<P2> = Vec::new();
        let (mut pos, mut theta) = ([0.0, 0.0], 0.0);
        let mut last_n: Option<Vec3> = None;
        let mut rate = |t: f64, uv: P2, span: (f64, f64)| -> Result<(f64, f64), KernelError> {
            let (c1, c2, n) = at(t, uv, span)?;
            let n = n.or(last_n);
            last_n = n;
            let s2 = c1.length_squared();
            let turn = match n {
                Some(n) if s2 > 0.0 => c2.dot(n.cross(c1)) / s2,
                _ => 0.0,
            };
            Ok((s2.sqrt(), turn))
        };
        if !samples.is_empty() {
            points.push(pos);
            at_samples.push(pos);
            for w in samples.windows(2) {
                let [(ta, uva), (tb, _)] = w else { continue };
                // Split at the curve's knots between the two samples, in
                // the order the samples run: the geodesic curvature jumps
                // there (a cubic Hermite spline has no second derivative
                // across its knots), which a rule across them does not
                // integrate (the sea kayak's chines, 2026-10-27: hundreds
                // of spans, their images 1e-3 off at 8 steps and still
                // 6e-6 at 256).
                let (lo, hi) = (ta.min(*tb), ta.max(*tb));
                let inner = breaks.iter().copied().filter(|k| *k > lo && *k < hi);
                let mut cuts: Vec<f64> = vec![*ta];
                if tb >= ta {
                    cuts.extend(inner);
                } else {
                    cuts.extend(inner.rev());
                }
                cuts.push(*tb);
                let whole = (tb - ta).abs();
                for c in cuts.windows(2) {
                    let [c0, c1] = c else { continue };
                    let span = (c0.min(*c1), c0.max(*c1));
                    let share = if whole > 0.0 {
                        (c1 - c0).abs() / whole
                    } else {
                        1.0
                    };
                    let n = ((sub as f64 * share).ceil() as usize).clamp(1, sub);
                    let h = (c1 - c0) / n as f64;
                    let mut here = rate(*c0, *uva, span)?;
                    for j in 0..n {
                        let t = c0 + h * j as f64;
                        let mid = rate(t + 0.5 * h, *uva, span)?;
                        let end = rate(if j + 1 == n { *c1 } else { t + h }, *uva, span)?;
                        let (w0, wm, w1) = (here.1, mid.1, end.1);
                        let theta_m = theta + h / 24.0 * (5.0 * w0 + 8.0 * wm - w1);
                        let theta_1 = theta + h / 6.0 * (w0 + 4.0 * wm + w1);
                        let dir = |th: f64, sp: f64| [sp * th.pcos(), sp * th.psin()];
                        let (d0, dm, d1) =
                            (dir(theta, here.0), dir(theta_m, mid.0), dir(theta_1, end.0));
                        let half = [
                            pos[0] + h / 24.0 * (5.0 * d0[0] + 8.0 * dm[0] - d1[0]),
                            pos[1] + h / 24.0 * (5.0 * d0[1] + 8.0 * dm[1] - d1[1]),
                        ];
                        pos = [
                            pos[0] + h / 6.0 * (d0[0] + 4.0 * dm[0] + d1[0]),
                            pos[1] + h / 6.0 * (d0[1] + 4.0 * dm[1] + d1[1]),
                        ];
                        theta = theta_1;
                        here = end;
                        mids.push(half);
                        points.push(pos);
                    }
                }
                at_samples.push(pos);
            }
        }
        // Laid onto the map's images of the samples.
        let images: Vec<P2> = samples.iter().map(|(_, uv)| p.flat(*uv)).collect();
        let (motion, residual) = Motion2::fit(&at_samples, &images)
            .ok_or_else(|| refused(format!("an edge of {} has no points", what())))?;
        if residual > 0.5 * tol {
            // Finer steps first: an edge whose geodesic curvature varies
            // within a step (a fitted curve's knots closer than its
            // samples) is integrated finer before it is refused.
            if sub < MAX_SUB {
                sub *= 2;
                continue;
            }
            return Err(refused(format!(
                "the flat image of an edge of {} departs from the pattern's map by {residual:e} (more than half the tolerance {tol}): the face is not laid out isometrically there",
                what()
            )));
        }
        let points: Vec<P2> = points.iter().map(|q| motion.apply(*q)).collect();
        let mids: Vec<P2> = mids.iter().map(|q| motion.apply(*q)).collect();
        let Some(mut curve) = spline_through(&points, &mids, edge, p.face, length_3d, tol)? else {
            return Ok(None);
        };
        if curve.deviation <= 0.5 * tol || sub >= MAX_SUB {
            if curve.deviation > 0.5 * tol {
                return Err(refused(format!(
                    "the outline of {} could not be drawn within half the tolerance",
                    what()
                )));
            }
            // The gate on lengths, measured directly.
            if (curve.length_flat - curve.length_3d).abs() > tol {
                return Err(refused(format!(
                    "an edge of {} is {:e} long in the pattern and {:e} on the part: more than the tolerance {tol} apart",
                    what(),
                    curve.length_flat,
                    curve.length_3d
                )));
            }
            curve.deviation = curve.deviation.max(residual);
            return Ok(Some(curve));
        }
        sub *= 2;
    }
}

/// The cubic spline through `points` at chord-length parameters.
fn spline_through(
    points: &[P2],
    mids: &[P2],
    edge: Option<EdgeId>,
    face: FaceId,
    length_3d: f64,
    tol: f64,
) -> Result<Option<FlatCurve>, KernelError> {
    // Points that coincide (a pole of the edge's samples) are one.
    let mut pts: Vec<P2> = Vec::with_capacity(points.len());
    for p in points {
        if pts.last().is_none_or(|q| len2(sub2(*p, *q)) > 1e-3 * tol) {
            pts.push(*p);
        }
    }
    // A piece shorter than the tolerance (where a loop meets a cut) is
    // a point of the outline, not a curve.
    let total_len: f64 = pts
        .windows(2)
        .map(|w| match w {
            [a, b] => len2(sub2(*b, *a)),
            _ => 0.0,
        })
        .sum();
    if pts.len() < 2 || total_len <= tol {
        return Ok(None);
    }
    let mut params = vec![0.0];
    let mut total = 0.0;
    for w in pts.windows(2) {
        if let [a, b] = w {
            total += len2(sub2(*b, *a));
            params.push(total);
        }
    }
    for t in &mut params {
        *t /= total;
    }
    let degree = 3.min(pts.len() - 1);
    let values: Vec<Vec<f64>> = pts.iter().map(|p| vec![p[0], p[1]]).collect();
    let (knots, control) = interpolate(&params, degree, &values).map_err(KernelError::Geometry)?;
    let control: Vec<P2> = control
        .iter()
        .map(|c| {
            [
                c.first().copied().unwrap_or(0.0),
                c.get(1).copied().unwrap_or(0.0),
            ]
        })
        .collect();
    // The spline's own length, by quadrature.
    let lifted: Vec<Vec3> = control.iter().map(|c| Vec3::new(c[0], c[1], 0.0)).collect();
    let curve = craft3d_geom::NurbsCurve::polynomial(degree, knots.clone(), lifted)
        .map_err(KernelError::Geometry)?;
    let (t0, t1) = curve.domain();
    let length_flat =
        Rule::new(6).integrate(t0, t1, 4 * pts.len(), |t| curve.derivatives(t).1.length());
    // How far the spline is from the mapped middles between the points
    // (at the middle of each interval's parameters: a bound on its
    // distance from the curve).
    let mut deviation: f64 = 0.0;
    {
        for m in mids {
            // Every middle against the spline, whatever points were
            // merged: from the kept point nearest it, the nearest point of
            // the spline within the intervals beside that point (Newton).
            let j = pts
                .iter()
                .enumerate()
                .min_by(|x, y| len2(sub2(*x.1, *m)).total_cmp(&len2(sub2(*y.1, *m))))
                .map_or(0, |(j, _)| j);
            let a = params.get(j.saturating_sub(1)).copied().unwrap_or(0.0);
            let b = params.get(j + 1).copied().unwrap_or(1.0);
            let (a, b) = (&a, &b);
            {
                let mut t = params.get(j).copied().unwrap_or(0.5 * (a + b));
                for _ in 0..8 {
                    let (q, d) = curve.derivatives(t);
                    let dd = d.x * d.x + d.y * d.y;
                    if dd <= 0.0 {
                        break;
                    }
                    t = (t + ((m[0] - q.x) * d.x + (m[1] - q.y) * d.y) / dd).clamp(*a, *b);
                }
                let q = curve.eval(t);
                deviation = deviation.max(len2(sub2([q.x, q.y], *m)));
            }
        }
    }
    Ok(Some(FlatCurve {
        edge,
        face,
        spline: Spline2 {
            degree,
            knots,
            control,
        },
        length_3d,
        length_flat,
        deviation,
    }))
}

impl Development {
    /// The flat point of `p`, a point of face `face`.
    pub fn to_flat(&self, face: FaceId, p: Vec3) -> Result<P2, KernelError> {
        let placed = self
            .placed
            .iter()
            .find(|q| q.face == face)
            .ok_or_else(|| refused(format!("face {} is not in this unfold", face.0)))?;
        Ok(placed.flat(placed.params_of(p)?))
    }

    /// The face and the point of the part at the flat point `p`.
    pub fn to_3d(&self, p: P2) -> Result<(FaceId, Vec3), KernelError> {
        for placed in &self.placed {
            let local = placed.motion.inverse_apply(p);
            if let Some(uv) = invert(placed, local)
                && placed.inside(uv)
            {
                return Ok((placed.face, placed.surface.eval(uv[0], uv[1])));
            }
        }
        Err(refused(format!(
            "({}, {}) is not on the flat pattern",
            p.first().copied().unwrap_or(f64::NAN),
            p.get(1).copied().unwrap_or(f64::NAN)
        )))
    }

    /// The images of the edges of the part that bound the pattern.
    pub fn outline(&self) -> &[FlatCurve] {
        &self.curves
    }

    pub fn error(&self) -> Distortion {
        self.error
    }

    /// The pattern as one flat sheet face in the plane of `frame`: its x
    /// and y are the pattern's, its normal the side the faces' outward
    /// normals were on.
    pub fn flat_body(&self, frame: &Frame, tolerance: &Tolerance) -> Result<OpResult, KernelError> {
        fill(&self.profiles, frame, tolerance)
    }

    /// The faces laid flat, in order.
    pub fn faces(&self) -> Vec<FaceId> {
        self.placed.iter().map(|p| p.face).collect()
    }
}

/// The surface parameters of the flat point `p` of a layout: the cell
/// whose points lie around it, then Newton on the interpolation.
fn invert(placed: &Placed, p: P2) -> Option<P2> {
    let layout = &placed.layout;
    let at = |u: f64, v: f64| layout.at(&placed.surface, placed.outward, u, v);
    let (nu, nv) = (layout.nu, layout.nv);
    let width = nv + 1;
    let node = |i: usize, j: usize| layout.flat.get(i * width + j).copied();
    let mut best: Option<(f64, P2)> = None;
    for i in 0..nu {
        for j in 0..nv {
            let (Some(a), Some(b), Some(c), Some(d)) = (
                node(i, j),
                node(i + 1, j),
                node(i + 1, j + 1),
                node(i, j + 1),
            ) else {
                continue;
            };
            let cx = 0.25 * (a[0] + b[0] + c[0] + d[0]);
            let cy = 0.25 * (a[1] + b[1] + c[1] + d[1]);
            let reach = [a, b, c, d]
                .iter()
                .map(|q| len2(sub2(*q, [cx, cy])))
                .fold(0.0, f64::max);
            let dist = len2(sub2(p, [cx, cy]));
            if dist <= 1.5 * reach && best.as_ref().is_none_or(|(bd, _)| dist < *bd) {
                let uv = [
                    layout.u.0 + (layout.u.1 - layout.u.0) * (i as f64 + 0.5) / nu as f64,
                    layout.v.0 + (layout.v.1 - layout.v.0) * (j as f64 + 0.5) / nv as f64,
                ];
                best = Some((dist, uv));
            }
        }
    }
    let (_, mut uv) = best?;
    let (hu, hv) = (
        1e-6 * (layout.u.1 - layout.u.0),
        1e-6 * (layout.v.1 - layout.v.0),
    );
    for _ in 0..50 {
        let f = at(uv[0], uv[1]);
        let r = sub2(f, p);
        if len2(r) <= 1e-13 * (1.0 + len2(p)) {
            return Some(uv);
        }
        let fu = sub2(at(uv[0] + hu, uv[1]), at(uv[0] - hu, uv[1]));
        let fv = sub2(at(uv[0], uv[1] + hv), at(uv[0], uv[1] - hv));
        let (a, b, c, d) = (
            fu[0] / (2.0 * hu),
            fv[0] / (2.0 * hv),
            fu[1] / (2.0 * hu),
            fv[1] / (2.0 * hv),
        );
        let det = a * d - b * c;
        if det.is_nan() || det == 0.0 {
            return None;
        }
        uv[0] -= (d * r[0] - b * r[1]) / det;
        uv[1] -= (-c * r[0] + a * r[1]) / det;
    }
    let r = sub2(at(uv[0], uv[1]), p);
    (len2(r) <= 1e-9 * (1.0 + len2(p))).then_some(uv)
}
