//! Shared by the hunts of plates and sheet metal (`hunt_sheet_*.rs`):
//! outlines of lines, arcs and splines described exactly (each arc by its
//! centre, radius, start angle and sweep), drawn into a sketch turned,
//! moved and scaled; their area and perimeter worked out apart from the
//! kernel (Green's theorem on the exact curves, Gauss–Legendre on the
//! splines' spans, exact for their degree); points known inside or
//! outside the material; and the checks every result gets (valid,
//! measurable, its mesh closed and of the volume measured, not through
//! itself), with a tally of outcomes held to the cases known wrong.

// Test support: a failure here is a failed test.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::boolean_common::Rng;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{Place, classify_points};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, check, mass_bound};
use std::f64::consts::{PI, TAU};

pub type P2 = [f64; 2];

pub fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// One piece of a loop, in the loop's own direction, exactly.
#[derive(Clone, Debug)]
pub enum Seg {
    Line(P2, P2),
    /// About `c` with radius `r`, from the angle `a0` by the signed sweep
    /// `sweep` (positive counter-clockwise); `from` and `to` its ends as
    /// computed once (shared bit for bit with the neighbours).
    Arc {
        c: P2,
        r: f64,
        a0: f64,
        sweep: f64,
        from: P2,
        to: P2,
    },
    /// A whole circle (a loop of its own), counter-clockwise when `ccw`.
    Circle {
        c: P2,
        r: f64,
        ccw: bool,
    },
    /// A clamped polynomial B-spline, run from its first control point to
    /// its last when not `back`, else the other way.
    Spline {
        degree: usize,
        knots: Vec<f64>,
        control: Vec<P2>,
        back: bool,
    },
}

pub fn polar(c: P2, r: f64, a: f64) -> P2 {
    [c[0] + r * a.pcos(), c[1] + r * a.psin()]
}

/// The arc about `c` of radius `r` from angle `a0` by `sweep`, its ends
/// `from` and `to` given (so they can be shared with the neighbours).
pub fn arc(c: P2, r: f64, a0: f64, sweep: f64, from: P2, to: P2) -> Seg {
    Seg::Arc {
        c,
        r,
        a0,
        sweep,
        from,
        to,
    }
}

/// The value and the derivative of a clamped B-spline at `t` (de Boor).
pub fn bspline(degree: usize, knots: &[f64], control: &[P2], t: f64) -> (P2, P2) {
    let n = control.len();
    let lo = knots[degree];
    let hi = knots[n];
    let t = t.clamp(lo, hi);
    let mut span = degree;
    while span + 1 < n && knots[span + 1] <= t {
        span += 1;
    }
    let point = |deg: usize, knots: &[f64], ctrl: &[P2], span: usize| -> P2 {
        let mut d: Vec<P2> = (0..=deg).map(|j| ctrl[span - deg + j]).collect();
        for r in 1..=deg {
            for j in (r..=deg).rev() {
                let i = span - deg + j;
                let den = knots[i + deg + 1 - r] - knots[i];
                let a = if den == 0.0 {
                    0.0
                } else {
                    (t - knots[i]) / den
                };
                d[j] = [
                    (1.0 - a) * d[j - 1][0] + a * d[j][0],
                    (1.0 - a) * d[j - 1][1] + a * d[j][1],
                ];
            }
        }
        d[deg]
    };
    let p = point(degree, knots, control, span);
    // The derivative: a spline of degree − 1 on the inner knots.
    let dctrl: Vec<P2> = (0..n - 1)
        .map(|i| {
            let den = knots[i + degree + 1] - knots[i + 1];
            let f = if den == 0.0 { 0.0 } else { degree as f64 / den };
            [
                f * (control[i + 1][0] - control[i][0]),
                f * (control[i + 1][1] - control[i][1]),
            ]
        })
        .collect();
    let dknots = &knots[1..knots.len() - 1];
    let d = if degree == 0 {
        [0.0, 0.0]
    } else {
        point(degree - 1, dknots, &dctrl, span - 1)
    };
    (p, d)
}

/// Gauss–Legendre nodes and weights on [−1, 1], 6 points (exact for
/// polynomials of degree up to 11).
const GAUSS: [(f64, f64); 6] = [
    (-0.932_469_514_203_152, 0.171_324_492_379_170),
    (-0.661_209_386_466_265, 0.360_761_573_048_139),
    (-0.238_619_186_083_197, 0.467_913_934_572_691),
    (0.238_619_186_083_197, 0.467_913_934_572_691),
    (0.661_209_386_466_265, 0.360_761_573_048_139),
    (0.932_469_514_203_152, 0.171_324_492_379_170),
];

impl Seg {
    pub fn start(&self) -> P2 {
        match self {
            Seg::Line(a, _) => *a,
            Seg::Arc { from, .. } => *from,
            Seg::Circle { c, r, .. } => [c[0] + r, c[1]],
            Seg::Spline { control, back, .. } => {
                if *back {
                    *control.last().unwrap()
                } else {
                    control[0]
                }
            }
        }
    }

    pub fn end(&self) -> P2 {
        match self {
            Seg::Line(_, b) => *b,
            Seg::Arc { to, .. } => *to,
            Seg::Circle { c, r, .. } => [c[0] + r, c[1]],
            Seg::Spline { control, back, .. } => {
                if *back {
                    control[0]
                } else {
                    *control.last().unwrap()
                }
            }
        }
    }

    /// The point and the unit tangent (in the loop's direction) at the
    /// fraction `s` of the piece.
    pub fn at(&self, s: f64) -> (P2, P2) {
        match self {
            Seg::Line(a, b) => {
                let d = [b[0] - a[0], b[1] - a[1]];
                let l = d[0].phypot(d[1]);
                ([a[0] + s * d[0], a[1] + s * d[1]], [d[0] / l, d[1] / l])
            }
            Seg::Arc {
                c, r, a0, sweep, ..
            } => {
                let a = a0 + s * sweep;
                let sign = sweep.signum();
                (polar(*c, *r, a), [-sign * a.psin(), sign * a.pcos()])
            }
            Seg::Circle { c, r, ccw } => {
                let a = s * TAU;
                let sign = if *ccw { 1.0 } else { -1.0 };
                let a = sign * a;
                (polar(*c, *r, a), [-sign * a.psin(), sign * a.pcos()])
            }
            Seg::Spline {
                degree,
                knots,
                control,
                back,
            } => {
                let (lo, hi) = (knots[*degree], knots[control.len()]);
                let s = if *back { 1.0 - s } else { s };
                let (p, d) = bspline(*degree, knots, control, lo + s * (hi - lo));
                let l = d[0].phypot(d[1]);
                let sign = if *back { -1.0 } else { 1.0 };
                (p, [sign * d[0] / l, sign * d[1] / l])
            }
        }
    }

    /// `∮ (x dy − y dx) / 2` along the piece, about the point `o`.
    pub fn green(&self, o: P2) -> f64 {
        match self {
            Seg::Line(a, b) => {
                let (ax, ay, bx, by) = (a[0] - o[0], a[1] - o[1], b[0] - o[0], b[1] - o[1]);
                0.5 * (ax * by - bx * ay)
            }
            Seg::Arc {
                c, r, a0, sweep, ..
            } => {
                let (cx, cy) = (c[0] - o[0], c[1] - o[1]);
                let b = a0 + sweep;
                0.5 * (r * cx * (b.psin() - a0.psin()) - r * cy * (b.pcos() - a0.pcos())
                    + r * r * sweep)
            }
            Seg::Circle { c, r, ccw } => {
                let _ = c;
                let sign = if *ccw { 1.0 } else { -1.0 };
                sign * PI * r * r
            }
            Seg::Spline {
                degree,
                knots,
                control,
                back,
            } => {
                let mut sum = 0.0;
                for i in *degree..control.len() {
                    let (a, b) = (knots[i], knots[i + 1]);
                    if b <= a {
                        continue;
                    }
                    for (x, w) in GAUSS {
                        let t = 0.5 * (a + b) + 0.5 * (b - a) * x;
                        let (p, d) = bspline(*degree, knots, control, t);
                        let (px, py) = (p[0] - o[0], p[1] - o[1]);
                        sum += w * 0.5 * (b - a) * 0.5 * (px * d[1] - py * d[0]);
                    }
                }
                if *back { -sum } else { sum }
            }
        }
    }

    pub fn length(&self) -> f64 {
        match self {
            Seg::Line(a, b) => (b[0] - a[0]).phypot(b[1] - a[1]),
            Seg::Arc { r, sweep, .. } => r * sweep.abs(),
            Seg::Circle { r, .. } => TAU * r,
            Seg::Spline {
                degree,
                knots,
                control,
                ..
            } => {
                let mut sum = 0.0;
                for i in *degree..control.len() {
                    let (a, b) = (knots[i], knots[i + 1]);
                    if b <= a {
                        continue;
                    }
                    // Each span in 16 for the speed's square root.
                    for j in 0..16 {
                        let (aa, bb) = (
                            a + (b - a) * f64::from(j) / 16.0,
                            a + (b - a) * f64::from(j + 1) / 16.0,
                        );
                        for (x, w) in GAUSS {
                            let t = 0.5 * (aa + bb) + 0.5 * (bb - aa) * x;
                            let (_, d) = bspline(*degree, knots, control, t);
                            sum += w * 0.5 * (bb - aa) * d[0].phypot(d[1]);
                        }
                    }
                }
                sum
            }
        }
    }
}

/// A shape: its outer loop (counter-clockwise) and holes (clockwise), in
/// its own axes at scale 1, with what it is.
#[derive(Clone, Debug)]
pub struct Shape {
    pub what: String,
    pub outer: Vec<Seg>,
    pub holes: Vec<Vec<Seg>>,
    /// A disc about this centre of this radius lies in the material, off
    /// every hole (points are drawn in it).
    pub inner: (P2, f64),
    /// The hole discs (centre and a radius that holds the hole).
    pub hole_discs: Vec<(P2, f64)>,
    /// The smallest width of any part (points are moved off the edges by
    /// a small share of it).
    pub feature: f64,
}

impl Shape {
    pub fn loops(&self) -> impl Iterator<Item = &Vec<Seg>> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }

    pub fn area(&self) -> f64 {
        let o = self.inner.0;
        self.loops().flatten().map(|s| s.green(o)).sum()
    }

    pub fn perimeter(&self) -> f64 {
        self.loops().flatten().map(Seg::length).sum()
    }

    pub fn pieces(&self) -> usize {
        self.loops().map(Vec::len).sum()
    }

    pub fn has_spline(&self) -> bool {
        self.loops()
            .flatten()
            .any(|s| matches!(s, Seg::Spline { .. }))
    }
}

/// Where a shape is drawn: turned by `turn` about its own origin, scaled
/// by `k` and moved to `at`, in the sketch's axes.
#[derive(Clone, Copy, Debug)]
pub struct Placing {
    pub turn: f64,
    pub k: f64,
    pub at: P2,
    /// Lines drawn from their end to their start.
    pub flip_lines: bool,
}

impl Placing {
    pub fn point(&self, p: P2) -> P2 {
        let (s, c) = (self.turn.psin(), self.turn.pcos());
        [
            self.at[0] + self.k * (c * p[0] - s * p[1]),
            self.at[1] + self.k * (s * p[0] + c * p[1]),
        ]
    }

    pub fn vector(&self, d: P2) -> P2 {
        let (s, c) = (self.turn.psin(), self.turn.pcos());
        [c * d[0] - s * d[1], s * d[0] + c * d[1]]
    }
}

/// The sketch curves of `shape` drawn by `placing`, ids from `first`.
pub fn sketch(shape: &Shape, placing: &Placing, first: u64) -> Vec<SketchCurve> {
    let mut out = Vec::new();
    for (id, seg) in (first..).zip(shape.loops().flatten()) {
        let curve = match seg {
            Seg::Line(a, b) => {
                let (a, b) = (placing.point(*a), placing.point(*b));
                if placing.flip_lines {
                    Curve2::Line { start: b, end: a }
                } else {
                    Curve2::Line { start: a, end: b }
                }
            }
            Seg::Arc {
                c, sweep, from, to, ..
            } => {
                let (c, from, to) = (placing.point(*c), placing.point(*from), placing.point(*to));
                if *sweep > 0.0 {
                    Curve2::Arc {
                        center: c,
                        start: from,
                        end: to,
                    }
                } else {
                    Curve2::Arc {
                        center: c,
                        start: to,
                        end: from,
                    }
                }
            }
            Seg::Circle { c, r, .. } => Curve2::Circle {
                center: placing.point(*c),
                radius: r * placing.k,
            },
            Seg::Spline {
                degree,
                knots,
                control,
                ..
            } => Curve2::Spline(Spline2 {
                degree: *degree,
                knots: knots.clone(),
                control: control.iter().map(|p| placing.point(*p)).collect(),
            }),
        };
        out.push(SketchCurve { id, curve });
    }
    out
}

// ---------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------

/// A closed loop through `points` (counter-clockwise), lines.
pub fn polygon_loop(points: &[P2]) -> Vec<Seg> {
    (0..points.len())
        .map(|i| Seg::Line(points[i], points[(i + 1) % points.len()]))
        .collect()
}

/// A loop reversed (its direction and every piece's).
pub fn reversed(segs: &[Seg]) -> Vec<Seg> {
    segs.iter()
        .rev()
        .map(|s| match s {
            Seg::Line(a, b) => Seg::Line(*b, *a),
            Seg::Arc {
                c,
                r,
                a0,
                sweep,
                from,
                to,
            } => Seg::Arc {
                c: *c,
                r: *r,
                a0: a0 + sweep,
                sweep: -sweep,
                from: *to,
                to: *from,
            },
            Seg::Circle { c, r, ccw } => Seg::Circle {
                c: *c,
                r: *r,
                ccw: !ccw,
            },
            Seg::Spline {
                degree,
                knots,
                control,
                back,
            } => Seg::Spline {
                degree: *degree,
                knots: knots.clone(),
                control: control.clone(),
                back: !back,
            },
        })
        .collect()
}

/// A loop moved by `d`.
pub fn moved(segs: &[Seg], d: P2) -> Vec<Seg> {
    let m = |p: &P2| [p[0] + d[0], p[1] + d[1]];
    segs.iter()
        .map(|s| match s {
            Seg::Line(a, b) => Seg::Line(m(a), m(b)),
            Seg::Arc {
                c,
                r,
                a0,
                sweep,
                from,
                to,
            } => Seg::Arc {
                c: m(c),
                r: *r,
                a0: *a0,
                sweep: *sweep,
                from: m(from),
                to: m(to),
            },
            Seg::Circle { c, r, ccw } => Seg::Circle {
                c: m(c),
                r: *r,
                ccw: *ccw,
            },
            Seg::Spline {
                degree,
                knots,
                control,
                back,
            } => Seg::Spline {
                degree: *degree,
                knots: knots.clone(),
                control: control.iter().map(m).collect(),
                back: *back,
            },
        })
        .collect()
}

/// A star-shaped polygon about the origin: `n` corners at random angles
/// and radii in `[r_lo, r_hi]`.
pub fn star(rng: &mut Rng, n: usize, r_lo: f64, r_hi: f64) -> Vec<P2> {
    let mut angles: Vec<f64> = (0..n)
        .map(|i| {
            // Three corners hold the centre only while no gap between them
            // is half a turn or more.
            let (lo, hi) = if n == 3 { (0.35, 0.65) } else { (0.15, 0.85) };
            (i as f64 + rng.between(lo, hi)) * TAU / n as f64
        })
        .collect();
    angles.sort_by(f64::total_cmp);
    angles
        .iter()
        .map(|a| polar([0.0, 0.0], rng.between(r_lo, r_hi), *a))
        .collect()
}

/// A circle about `c` of radius `r` cut into arcs at the angles `cuts`
/// (increasing, within one turn of the first), counter-clockwise.
pub fn split_circle(c: P2, r: f64, cuts: &[f64]) -> Vec<Seg> {
    let points: Vec<P2> = cuts.iter().map(|a| polar(c, r, *a)).collect();
    let n = cuts.len();
    (0..n)
        .map(|i| {
            let a0 = cuts[i];
            let a1 = if i + 1 < n {
                cuts[i + 1]
            } else {
                cuts[0] + TAU
            };
            arc(c, r, a0, a1 - a0, points[i], points[(i + 1) % n])
        })
        .collect()
}

/// `n` cuts of a turn at random angles from `start`, at least `gap` apart.
pub fn cuts(rng: &mut Rng, n: usize, start: f64) -> Vec<f64> {
    loop {
        let mut a: Vec<f64> = (0..n).map(|_| rng.between(0.0, TAU)).collect();
        a.sort_by(f64::total_cmp);
        let gap_ok = (0..n).all(|i| {
            let next = if i + 1 < n { a[i + 1] } else { a[0] + TAU };
            next - a[i] > 0.05
        });
        if gap_ok {
            return a.iter().map(|x| x + start).collect();
        }
    }
}

/// A rectangle `w` × `h` about the origin with its corners rounded to `r`
/// (`r` below half of each), counter-clockwise, its arcs tangent to the
/// lines. Area `w h − (4 − π) r²`.
pub fn rounded_rect(w: f64, h: f64, r: f64) -> Vec<Seg> {
    let (x, y) = (0.5 * w, 0.5 * h);
    let centres = [
        [x - r, -y + r],
        [x - r, y - r],
        [-x + r, y - r],
        [-x + r, -y + r],
    ];
    let mut segs = Vec::new();
    // Corner i turns from angle −90 + 90 i by 90 degrees.
    let starts = [-0.5 * PI, 0.0, 0.5 * PI, PI];
    let ends: Vec<(P2, P2)> = (0..4)
        .map(|i| {
            (
                polar(centres[i], r, starts[i]),
                polar(centres[i], r, starts[i] + 0.5 * PI),
            )
        })
        .collect();
    for i in 0..4 {
        segs.push(arc(
            centres[i],
            r,
            starts[i],
            0.5 * PI,
            ends[i].0,
            ends[i].1,
        ));
        segs.push(Seg::Line(ends[i].1, ends[(i + 1) % 4].0));
    }
    segs
}

/// A slot (stadium) of half-length `l` (between the centres) and radius
/// `r` about the origin along x, counter-clockwise; the half circles
/// split into `split` arcs each. Area `4 l r + π r²`.
pub fn slot(l: f64, r: f64, split: usize) -> Vec<Seg> {
    let mut segs = Vec::new();
    for (c, a0) in [([l, 0.0], -0.5 * PI), ([-l, 0.0], 0.5 * PI)] {
        let n = split.max(1);
        let pts: Vec<P2> = (0..=n)
            .map(|j| polar(c, r, a0 + PI * j as f64 / n as f64))
            .collect();
        for j in 0..n {
            segs.push(arc(
                c,
                r,
                a0 + PI * j as f64 / n as f64,
                PI / n as f64,
                pts[j],
                pts[j + 1],
            ));
        }
    }
    // Lines between the half circles.
    let mut out = Vec::new();
    let half = segs.len() / 2;
    for (i, s) in segs.iter().enumerate() {
        out.push(s.clone());
        if i + 1 == half {
            out.push(Seg::Line(s.end(), segs[half].start()));
        }
    }
    let last = out.last().unwrap().end();
    out.push(Seg::Line(last, out[0].start()));
    out
}

/// A gear of `teeth` teeth about the origin: root circle `rr`, tip circle
/// `rt`, each tooth's flanks radial lines, the tooth's tip an arc of the
/// tip circle over `tip` of the pitch angle, the gap between two teeth two
/// arcs of the root circle meeting in its middle (a chained ring: root
/// arcs of neighbouring teeth meeting end to end), counter-clockwise.
pub fn gear(teeth: usize, rr: f64, rt: f64, tip: f64, phase: f64) -> Vec<Seg> {
    let pitch = TAU / teeth as f64;
    let half_tip = 0.5 * tip * pitch;
    let mut segs = Vec::new();
    let o = [0.0, 0.0];
    // Tooth i spans the angles t0[i]..t1[i]; the gap after it runs to
    // t0[i + 1] (a turn on for the last).
    let t0: Vec<f64> = (0..teeth)
        .map(|i| phase + pitch * i as f64 - half_tip)
        .collect();
    let t1: Vec<f64> = t0.iter().map(|a| a + 2.0 * half_tip).collect();
    let tip0: Vec<P2> = t0.iter().map(|a| polar(o, rt, *a)).collect();
    let tip1: Vec<P2> = t1.iter().map(|a| polar(o, rt, *a)).collect();
    let root0: Vec<P2> = t0.iter().map(|a| polar(o, rr, *a)).collect();
    let root1: Vec<P2> = t1.iter().map(|a| polar(o, rr, *a)).collect();
    for i in 0..teeth {
        let j = (i + 1) % teeth;
        let g0 = t1[i];
        let g1 = if j == 0 { t0[0] + TAU } else { t0[j] };
        let gm = 0.5 * (g0 + g1);
        let root_m = polar(o, rr, gm);
        segs.push(arc(o, rt, t0[i], t1[i] - t0[i], tip0[i], tip1[i]));
        segs.push(Seg::Line(tip1[i], root1[i]));
        segs.push(arc(o, rr, g0, gm - g0, root1[i], root_m));
        segs.push(arc(o, rr, gm, g1 - gm, root_m, root0[j]));
        segs.push(Seg::Line(root0[j], tip0[j]));
    }
    segs
}

/// A gear sector: the pie from the origin between the angles `0` and
/// `span`, its arc carrying `teeth` teeth as [`gear`]'s (root `rr`, tip
/// `rt`), counter-clockwise.
pub fn gear_sector(teeth: usize, span: f64, rr: f64, rt: f64, tip: f64) -> Vec<Seg> {
    let pitch = span / teeth as f64;
    let half_tip = 0.5 * tip * pitch;
    let o = [0.0, 0.0];
    let mut segs = Vec::new();
    // From the origin out along angle 0 to the root, then gap-halves and
    // teeth: the root starts with half a gap.
    let start = polar(o, rr, 0.0);
    segs.push(Seg::Line(o, start));
    let mut at = start;
    let mut a = 0.0;
    for i in 0..teeth {
        let m = pitch * (i as f64 + 0.5);
        let (t0, t1) = (m - half_tip, m + half_tip);
        // Root from a to t0 in two arcs meeting in the middle.
        let mid = 0.5 * (a + t0);
        let pm = polar(o, rr, mid);
        let p0 = polar(o, rr, t0);
        segs.push(arc(o, rr, a, mid - a, at, pm));
        segs.push(arc(o, rr, mid, t0 - mid, pm, p0));
        let q0 = polar(o, rt, t0);
        let q1 = polar(o, rt, t1);
        let p1 = polar(o, rr, t1);
        segs.push(Seg::Line(p0, q0));
        segs.push(arc(o, rt, t0, t1 - t0, q0, q1));
        segs.push(Seg::Line(q1, p1));
        at = p1;
        a = t1;
    }
    let mid = 0.5 * (a + span);
    let pm = polar(o, rr, mid);
    let pe = polar(o, rr, span);
    segs.push(arc(o, rr, a, mid - a, at, pm));
    segs.push(arc(o, rr, mid, span - mid, pm, pe));
    segs.push(Seg::Line(pe, o));
    segs
}

/// A rectangle `w` × `h` from `(−w/2, −h/2)` whose top runs as a clamped
/// cubic spline through random control points between `0.6 h` and
/// `1.4 h` of the half height, counter-clockwise.
pub fn spline_top(rng: &mut Rng, w: f64, h: f64, n: usize) -> Vec<Seg> {
    let (x, y) = (0.5 * w, 0.5 * h);
    let n = n.max(4);
    let control: Vec<P2> = (0..n)
        .map(|i| {
            let px = x - w * i as f64 / (n - 1) as f64;
            let py = if i == 0 || i + 1 == n {
                y
            } else {
                y * rng.between(0.6, 1.4)
            };
            [px, py]
        })
        .collect();
    let degree = 3;
    let inner = n - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    for i in 1..=inner {
        knots.push(i as f64 / (inner + 1) as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    let a = [-x, -y];
    let b = [x, -y];
    vec![
        Seg::Line(a, b),
        Seg::Line(b, control[0]),
        Seg::Spline {
            degree,
            knots,
            control: control.clone(),
            back: false,
        },
        Seg::Line(*control.last().unwrap(), a),
    ]
}

/// A "D": an arc of radius `r` about the origin from −80 to the angle
/// `at` (degrees), continued from there by an arc of radius `r (1 + eps)`
/// whose tangent is turned by `delta` (radians, outward for a positive
/// one) to the angle 80 about its own centre, closed by a line: a real
/// crease of the angle `delta` (none at 0, 0) between arcs of nearly one
/// radius.
pub fn crease_d(r: f64, at: f64, eps: f64, delta: f64) -> Vec<Seg> {
    let c1 = [0.0, 0.0];
    let a0 = (-80.0f64).to_radians();
    let ap = at.to_radians();
    let start = polar(c1, r, a0);
    let p = polar(c1, r, ap);
    // The second centre: from p along the inward normal turned by delta.
    let r2 = r * (1.0 + eps);
    let inward = ap + PI + delta;
    let c2 = polar(p, r2, inward);
    // The angle of p about c2.
    let b0 = ap + delta;
    let b1 = 80.0f64.to_radians();
    let end = polar(c2, r2, b1);
    vec![
        arc(c1, r, a0, ap - a0, start, p),
        arc(c2, r2, b0, b1 - b0, p, end),
        Seg::Line(end, start),
    ]
}

// ---------------------------------------------------------------------
// Measures
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Measured {
    pub volume: f64,
    pub area: f64,
    pub bound: f64,
}

pub fn measured(body: &Body) -> Option<Measured> {
    let (m, bound) = mass_bound(body).ok()?;
    (m.volume.is_finite() && m.area.is_finite()).then_some(Measured {
        volume: m.volume,
        area: m.area,
        bound: bound.volume,
    })
}

pub fn bounds(body: &Body) -> (Vec3, Vec3) {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    let mut add = |p: Vec3| {
        lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    };
    for v in &body.vertices {
        add(v.point);
    }
    for e in &body.edges {
        for i in 0..=8 {
            add(e
                .curve
                .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(i) / 8.0));
        }
    }
    // A body without edges (a hollow ball or ring: whole spheres and tori,
    // built since 2026-11-15) has nothing to sample: its box, which for
    // whole spheres and tori is theirs exactly.
    if body.edges.is_empty()
        && let Ok(b) = body.bbox()
        && let Some((blo, bhi)) = b.corners()
    {
        add(blo);
        add(bhi);
    }
    (lo, hi)
}

pub fn size_of(body: &Body) -> f64 {
    let (lo, hi) = bounds(body);
    (hi - lo).length()
}

/// The volume of the mesh of `body` at the chordal tolerance `chord`,
/// welded and closed.
pub fn mesh_volume(body: &Body, chord: f64) -> Result<f64, String> {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .map_err(|e| format!("not meshed: {e}"))?;
    let welded = mesh.weld().map_err(|e| format!("not welded: {e:?}"))?;
    if !welded.is_closed() {
        return Err("the mesh is not closed".into());
    }
    Ok(welded.volume())
}

pub fn crosses_itself(body: &Body, tol: &Tolerance) -> bool {
    body_self_intersections(body, &SelfCheckParams::new(tol.linear)).is_ok_and(|c| !c.is_clean())
}

pub fn any_freeform(body: &Body) -> bool {
    body.faces
        .iter()
        .any(|f| matches!(f.surface, Surface::Nurbs(_)))
}

/// What the oracles found of one result.
#[derive(Default, Debug)]
pub struct Found {
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub invalid: bool,
    pub unmeshable: bool,
}

/// Valid, measurable, of positive volume, its mesh closed and of the
/// volume measured. The measure, where there is one.
pub fn sound(body: &Body, tol: &Tolerance, found: &mut Found) -> Option<Measured> {
    let defects = check(body, tol);
    if !defects.is_empty() {
        found.invalid = true;
        found
            .problems
            .push(format!("not valid: {:?}", defects.first()));
        return None;
    }
    let size = size_of(body);
    let Some(m) = measured(body) else {
        if let Err(e) = mesh_volume(body, 1e-3 * size) {
            found.unmeshable = true;
            found.problems.push(format!(
                "passes the checker but can be neither measured nor meshed: {e}"
            ));
        } else {
            found.problems.push("cannot be measured".into());
        }
        return None;
    };
    if m.volume <= 0.0 {
        found
            .problems
            .push(format!("a volume not above zero: {:e}", m.volume));
        return None;
    }
    let chord = 1e-3 * size;
    match mesh_volume(body, chord) {
        Ok(v) => {
            let bound = 2.0 * chord * m.area + m.bound;
            if (v - m.volume).abs() > bound {
                found.problems.push(format!(
                    "the mesh's volume {v:e} against the measured {:e} (bound {bound:e})",
                    m.volume
                ));
            }
        }
        Err(e) => {
            let others: Vec<String> = [1e-2, 1e-4]
                .iter()
                .map(|f| match mesh_volume(body, f * size) {
                    Ok(v) => format!("at {f}: {v:e} for {:e}", m.volume),
                    Err(e) => format!("at {f}: {e}"),
                })
                .collect();
            if others.iter().all(|o| o.contains("not meshed")) {
                found.unmeshable = true;
                found
                    .problems
                    .push(format!("passes the checker but cannot be meshed: {e}"));
            } else {
                found.notes.push(format!("mesh: {e}; {others:?}"));
            }
        }
    }
    Some(m)
}

/// The winding number of the closed mesh of `body` (chord `chord`) about
/// `p`: the sum of its triangles' signed solid angles over 4π (Van
/// Oosterom and Strackee), 1 inside and 0 outside; `None` when it cannot
/// be meshed. A second means for a point the rays place wrong.
pub fn mesh_winding(body: &Body, chord: f64, p: Vec3) -> Option<f64> {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        },
    )
    .ok()?;
    let mut sum = 0.0;
    for t in 0..mesh.triangles.len() {
        let [a, b, c] = mesh.corners(t)?;
        let (a, b, c) = (a - p, b - p, c - p);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let num = a.dot(b.cross(c));
        let den = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
        sum += 2.0 * num.patan2(den);
    }
    Some(sum / (4.0 * PI))
}

/// Points classified against `body`, each against what it must be:
/// `(point, inside)`. The problems found (a point inside that is outside
/// or the other way round, the rays' answer checked again by the mesh's
/// winding number: a point the mesh agrees with the rays on is a
/// problem, one where the mesh says what it must be is a note of the
/// rays' own fault); unknowns counted as notes.
pub fn rays(body: &Body, points: &[(Vec3, bool)], tol: &Tolerance, found: &mut Found) {
    let ps: Vec<Vec3> = points.iter().map(|(p, _)| *p).collect();
    let Ok(places) = classify_points(body, &ps, tol) else {
        found.notes.push("the rays could not classify".into());
        return;
    };
    let mut wrong = 0;
    let mut rays_fault = 0;
    let mut unknown = 0;
    let mut first = None;
    let mut first_fault = None;
    let chord = 1e-4 * size_of(body);
    for (index, ((p, inside), place)) in points.iter().zip(&places).enumerate() {
        let got = match place {
            Place::Unknown | Place::OnBoundary => {
                unknown += 1;
                continue;
            }
            Place::Inside => true,
            Place::Outside => false,
        };
        if got == *inside {
            continue;
        }
        let w = mesh_winding(body, chord, *p);
        let mesh_says = w.map(|w| w > 0.5);
        if mesh_says == Some(*inside) {
            rays_fault += 1;
            first_fault.get_or_insert((*p, got, w, index));
        } else {
            wrong += 1;
            first.get_or_insert((*p, got, w, index));
        }
    }
    if wrong > 0 {
        let (p, got, w, index) = first.unwrap();
        found.problems.push(format!(
            "{wrong} of {} points classified wrong by rays and the mesh (first, number {index}, at {p:?}: rays inside {got}, mesh winding {w:?})",
            points.len()
        ));
    }
    if rays_fault > 0 {
        let (p, got, w, index) = first_fault.unwrap();
        found.notes.push(format!(
            "RAYS: {rays_fault} of {} points the rays place wrong where the mesh places them right (first, number {index}, at {p:?}: rays inside {got}, mesh winding {w:?})",
            points.len()
        ));
    }
    if unknown > 0 {
        found
            .notes
            .push(format!("{unknown} points unknown or on the boundary"));
    }
}

/// Points known about a plate of `shape` drawn by `placing` in `frame`
/// between the levels `lo` and `hi` along the frame's z: in the inner disc
/// at mid-level (inside) and past each skin (outside); beside the loops,
/// a small way off each piece into the material (inside) and out of it
/// (outside), at mid-level.
pub fn plate_points(
    rng: &mut Rng,
    shape: &Shape,
    placing: &Placing,
    frame: &Frame,
    (lo, hi): (f64, f64),
    count: usize,
) -> Vec<(Vec3, bool)> {
    let t = hi - lo;
    let mid = 0.5 * (lo + hi);
    let at = |p: P2, z: f64| {
        let q = placing.point(p);
        frame.point(q[0], q[1], z)
    };
    let mut out = Vec::new();
    let (c, rho) = shape.inner;
    for _ in 0..count {
        let a = rng.between(0.0, TAU);
        let r = rho * rng.unit().sqrt() * 0.95;
        let p = polar(c, r, a);
        if shape
            .hole_discs
            .iter()
            .any(|(hc, hr)| (p[0] - hc[0]).phypot(p[1] - hc[1]) < hr * 1.05)
        {
            continue;
        }
        out.push((at(p, mid), true));
        out.push((at(p, hi + 0.25 * t), false));
        out.push((at(p, lo - 0.25 * t), false));
    }
    let off = 0.02 * shape.feature;
    for seg in shape.loops().flatten() {
        let s = rng.between(0.25, 0.75);
        let (p, d) = seg.at(s);
        // Material on the left of the loop's direction.
        let left = [-d[1], d[0]];
        out.push((at([p[0] + off * left[0], p[1] + off * left[1]], mid), true));
        out.push((at([p[0] - off * left[0], p[1] - off * left[1]], mid), false));
    }
    out
}

// ---------------------------------------------------------------------
// The tally
// ---------------------------------------------------------------------

/// What happened to a case, as a letter: `B` built and held, `W` built and
/// seen wrong, `I` an invalid body returned, `M` a body the checker passes
/// but that cannot be meshed, `X` built and passing through itself (the
/// regeneration refuses it), `R` refused, `P` a panic, `S` no case set up.
pub struct Tally {
    pub outcomes: String,
    pub wrong: Vec<String>,
    pub notes: Vec<String>,
    pub refusals: std::collections::BTreeMap<String, usize>,
}

impl Default for Tally {
    fn default() -> Self {
        Self::new()
    }
}

impl Tally {
    pub fn new() -> Self {
        Tally {
            outcomes: String::new(),
            wrong: Vec::new(),
            notes: Vec::new(),
            refusals: std::collections::BTreeMap::new(),
        }
    }

    pub fn count(&self, c: char) -> usize {
        self.outcomes.chars().filter(|o| *o == c).count()
    }

    /// Counts a refusal by the start of its reason (numbers taken out).
    pub fn refused(&mut self, why: &str) {
        let short = shortened(why, 110);
        *self.refusals.entry(short).or_insert(0) += 1;
        self.outcomes.push('R');
    }

    pub fn summary(&self, name: &str) -> String {
        format!(
            "{name}: cases {}, built right {}, refused {}, wrong {}, invalid {}, unmeshable {}, through itself {}, panic {}, not set up {}",
            self.outcomes.len(),
            self.count('B'),
            self.count('R'),
            self.count('W'),
            self.count('I'),
            self.count('M'),
            self.count('X'),
            self.count('P'),
            self.count('S'),
        )
    }

    /// Prints the tally and holds it to `known`: the cases seen wrong that
    /// are known, by number from `start`; a known case that is no longer
    /// wrong has to come off the list.
    pub fn hold(&self, name: &str, start: usize, known: &[usize], least_built: usize) {
        println!("{}", self.summary(name));
        println!("{name}: outcomes from {start}: {}", self.outcomes);
        for (why, n) in &self.refusals {
            println!("  refused {n}x: {why}");
        }
        for line in &self.wrong {
            println!("  {line}");
        }
        for line in self.notes.iter().take(40) {
            println!("  note: {line}");
        }
        let mut unknown = Vec::new();
        for (i, c) in self.outcomes.chars().enumerate() {
            let n = start + i;
            let bad = matches!(c, 'W' | 'I' | 'M' | 'P');
            if bad && !known.contains(&n) {
                unknown.push(format!("case {n}: {c}"));
            }
            if !bad && known.contains(&n) {
                unknown.push(format!(
                    "case {n}: known wrong, now {c}: take it off the list"
                ));
            }
        }
        assert!(unknown.is_empty(), "{name}: {unknown:#?}");
        assert!(
            self.count('B') >= least_built,
            "{name}: too few built to say much ({} of at least {least_built})",
            self.count('B')
        );
    }

    /// Records a result: the letter from what was found.
    pub fn record(&mut self, case: usize, what: &str, found: &Found, crosses: bool) {
        let c = if found.invalid {
            'I'
        } else if found.unmeshable {
            'M'
        } else if !found.problems.is_empty() {
            if crosses { 'X' } else { 'W' }
        } else if crosses {
            'X'
        } else {
            'B'
        };
        if c != 'B' {
            self.wrong
                .push(format!("case {case} ({c}): {what}: {:?}", found.problems));
        }
        for n in &found.notes {
            self.notes.push(format!("case {case}: {n}"));
        }
        self.outcomes.push(c);
    }
}

/// A reason with every number written `#`, cut to `n` characters: what
/// refusals are grouped by.
pub fn shortened(why: &str, n: usize) -> String {
    let mut out = String::new();
    let mut in_number = false;
    let chars: Vec<char> = why.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let signed = c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        let part =
            c.is_ascii_digit() || signed || (in_number && matches!(c, '.' | 'e' | '-' | '+'));
        if part {
            if !in_number {
                out.push('#');
            }
        } else {
            out.push(c);
        }
        in_number = part;
    }
    out.chars().take(n).collect()
}

/// Runs `f`, a panic caught and returned as its message.
pub fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "a panic".into())
    })
}

/// The seed of case `n` of a sweep seeded `seed`.
pub fn case_seed(seed: u64, n: usize) -> u64 {
    seed ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// A frame at scale `k`: the world, or turned and moved some way off.
pub fn frame(rng: &mut Rng, k: f64) -> Frame {
    if rng.below(4) == 0 {
        return Frame::WORLD;
    }
    let far = if rng.below(3) == 0 { 1e4 } else { 200.0 };
    let origin = Vec3::new(
        rng.between(-far, far) * k,
        rng.between(-far, far) * k,
        rng.between(-far, far) * k,
    );
    let z = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let x = Vec3::new(
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
        rng.between(-1.0, 1.0),
    );
    let f = Frame::new(origin, z, x).unwrap_or(Frame::WORLD);
    if rng.below(2) == 0 {
        f.reversed_at(f.origin()).unwrap()
    } else {
        f
    }
}
