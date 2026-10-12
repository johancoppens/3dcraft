//! The general, numeric surface/surface intersection.
//!
//! One surface is walked in its parameters `(u, v)` (the *chart*), the
//! other is a signed distance `F` (the *field*). Their intersection is the
//! zero set of `f(u, v) = F(S(u, v))`, whose gradient is
//! `(n·S_u, n·S_v)` with `n` the field's unit normal.
//!
//! **Finding every branch.** The chart's rectangle is divided into cells
//! (a quadtree). A cell of half sizes `(hu, hv)` and centre `c` is
//!
//! - *empty* when `|F(S(c))| > Lu hu + Lv hv`: a distance is 1-Lipschitz
//!   and `Lu`, `Lv` bound the chart's speed, so `f` has no zero in it;
//! - *simple* when `|f_u(c)| > Muu hu + Muv hv` (or the same for `v`), with
//!   `M` bounding the second derivatives of `f`
//!   (`Muu = K Lu² + |S_uu|`, `K` the field's curvature): `f` is then
//!   strictly monotonic along u all over the cell, so the cell holds no
//!   point where both partial derivatives vanish, and the zero set crosses
//!   each of its two edges along u at most once, which the signs of `f` at
//!   the edge's ends detect;
//! - otherwise split in four, down to cells no larger than the linear
//!   tolerance (*floor* cells).
//!
//! A branch that no sign change detects would have to cross only edges
//! along which `f` is not monotonic. Following it through simple cells,
//! that makes it a closed loop inside cells that are all monotonic in the
//! same direction, which is impossible (a loop has a point where its
//! tangent has that direction, and there that partial derivative is zero),
//! or a piece that leaves the rectangle twice through one cell's edge on
//! the rectangle's border, which is why border edges are subdivided by the
//! same rule in one dimension. What is left passes through floor cells:
//! near points where the surfaces are tangent. There the critical point of
//! `f` is found by Newton iteration and classified by the Hessian: a
//! minimum or maximum is an isolated touch (reported as a point; a loop
//! smaller than a floor cell, twice the linear tolerance across, is
//! indistinguishable from it), a saddle is a point where two branches
//! cross (every arm gets a seed, and the march stops there), anything
//! flatter is refused.
//!
//! The derivative bounds are proofs for the analytic kinds and polynomial
//! Bézier patches as chart, and for the analytic kinds as field; for NURBS
//! charts and freeform fields they are estimates from samples.
//!
//! **Marching.** From each seed the branch is followed both ways by a
//! tangent step and Newton correction back onto `f = 0`, with the step
//! limited by the turn of the tangent. It ends where it closes, leaves the
//! chart's rectangle, runs off the edge of the other surface, or reaches a
//! crossing point or a cone's apex.
//!
//! **Fitting.** The points and tangents define a C¹ cubic (a Hermite
//! spline, as a NURBS with double knots, parametrised close to arc
//! length). Its distance to both surfaces is measured at three places per
//! span and points are added until it is within the tolerance.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::field::{Chart, Field, Sample};
use super::meet::{Meeting, MeetingCurve};
use crate::{Curve3, GeomError, NurbsCurve, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};

/// Cells looked at before the surfaces are declared too close to tangent
/// (or coincident) over too large an area.
const MAX_CELLS: usize = 400_000;
const MAX_FLOOR_CELLS: usize = 2_000;
const MAX_POINTS: usize = 16_000;
const MAX_BRANCHES: usize = 512;
const MAX_EVALS: usize = 40_000_000;
/// Deepest subdivision: the finest cell is `2^-MAX_LEVELS` of a base cell.
const MAX_LEVELS: u32 = 40;
/// Largest turn of the tangent in one step, in radians.
const MAX_TURN: f64 = 0.2;
/// Sine of the angle between the surfaces below which a point is not
/// marched through.
const MIN_SINE: f64 = 1e-8;

fn stalled() -> GeomError {
    GeomError::NoConvergence(
        "marching an intersection curve stalled (the surfaces are nearly tangent along it, or one of them has a crease)",
    )
}

/// Half the step, or the error that marching has stalled.
fn halved(h: f64, hmin: f64) -> Result<f64, GeomError> {
    if 0.5 * h < hmin {
        Err(stalled())
    } else {
        Ok(0.5 * h)
    }
}

fn too_many() -> GeomError {
    GeomError::NoConvergence(
        "the surfaces are tangent or nearly coincident over too large an area to intersect",
    )
}

#[derive(Clone, Copy, Debug)]
struct Eval {
    p: Vec3,
    su: Vec3,
    sv: Vec3,
    f: f64,
    dist: f64,
    n: Vec3,
    gu: f64,
    gv: f64,
}

/// A point of a branch: chart parameters (not wrapped), position, unit
/// tangent in the direction of travel and the same tangent in parameters.
#[derive(Clone, Copy, Debug)]
struct Pt {
    u: f64,
    v: f64,
    p: Vec3,
    t: Vec3,
    du: f64,
    dv: f64,
}

impl Pt {
    fn reversed(self) -> Pt {
        Pt {
            t: -self.t,
            du: -self.du,
            dv: -self.dv,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SingularKind {
    /// The surfaces touch and separate again: no curve.
    Touch,
    /// Two branches cross.
    Cross,
    /// The apex of the field's cone.
    Apex,
}

#[derive(Clone, Copy, Debug)]
struct Singular {
    u: f64,
    v: f64,
    p: Vec3,
    kind: SingularKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    Closed,
    Open,
}

struct Branch {
    pts: Vec<Pt>,
    closed: bool,
}

struct Grid {
    u0: f64,
    v0: f64,
    du: f64,
    dv: f64,
    nu: i64,
    nv: i64,
    base: i64,
}

impl Grid {
    fn node(&self, i: i64, j: i64) -> (f64, f64) {
        (self.u0 + self.du * i as f64, self.v0 + self.dv * j as f64)
    }
}

pub(super) struct Marcher {
    chart: Chart,
    field: Field,
    own: Field,
    tol: Tolerance,
    /// How close to `f = 0` a marched point is brought.
    on: f64,
    /// Distance from the field's surface beyond which a point with
    /// `f = 0` is past that surface's edge.
    cut: f64,
    hmax: f64,
    hmin: f64,
    evals: usize,
}

impl Marcher {
    pub(super) fn new(
        chart: Chart,
        field: Field,
        within: &Bbox3,
        tol: &Tolerance,
    ) -> Result<Self, GeomError> {
        let own = Field::new(&chart.surface)?;
        let (lo, hi) = within
            .corners()
            .ok_or(GeomError::InvalidParameter("an empty box"))?;
        let scale = lo.distance(hi);
        let magnitude = lo.length().max(hi.length()) + scale;
        let on = (1e-4 * tol.linear).max(64.0 * f64::EPSILON * magnitude);
        if !(scale > 0.0) || on > 0.02 * tol.linear {
            return Err(GeomError::InvalidParameter(
                "the box has no size, or the tolerance is finer than rounding allows at its size",
            ));
        }
        Ok(Self {
            chart,
            field,
            own,
            tol: *tol,
            on,
            cut: 0.25 * tol.linear,
            hmax: scale / 32.0,
            hmin: 4.0 * on,
            evals: 0,
        })
    }

    fn spent(&self) -> Result<(), GeomError> {
        if self.evals > MAX_EVALS {
            Err(GeomError::NoConvergence(
                "the intersection needs more evaluations than the cap allows",
            ))
        } else {
            Ok(())
        }
    }

    fn eval(&mut self, u: f64, v: f64) -> Option<Eval> {
        self.evals += 1;
        let (p, su, sv) = self.chart.surface.derivatives(u, v);
        let Sample {
            signed,
            dist,
            normal,
        } = self.field.sample(p)?;
        (su.is_finite() && sv.is_finite()).then_some(Eval {
            p,
            su,
            sv,
            f: signed,
            dist,
            n: normal,
            gu: normal.dot(su),
            gv: normal.dot(sv),
        })
    }

    fn inside(&self, u: f64, v: f64) -> bool {
        (self.chart.periodic_u || (u >= self.chart.u.0 && u <= self.chart.u.1))
            && (self.chart.periodic_v || (v >= self.chart.v.0 && v <= self.chart.v.1))
    }

    /// Newton back onto `f = 0`, moving the shortest way in space.
    fn correct(&mut self, mut u: f64, mut v: f64) -> Option<(Eval, f64, f64)> {
        for _ in 0..16 {
            let e = self.eval(u, v)?;
            if e.f.abs() <= self.on {
                return Some((e, u, v));
            }
            // w = G⁻¹ g with G the first fundamental form.
            let (a, b, c) = (e.su.dot(e.su), e.su.dot(e.sv), e.sv.dot(e.sv));
            let det = a * c - b * b;
            let (wu, wv) = if det > 1e-14 * a * c {
                ((c * e.gu - b * e.gv) / det, (a * e.gv - b * e.gu) / det)
            } else {
                (e.gu, e.gv)
            };
            let slope = e.gu * wu + e.gv * wv;
            if !(slope > 0.0) {
                return None;
            }
            u -= e.f * wu / slope;
            v -= e.f * wv / slope;
            if !(u.is_finite() && v.is_finite()) {
                return None;
            }
        }
        None
    }

    /// The point of a branch at an evaluated place, travelling the way
    /// `sign` says.
    fn point(e: &Eval, u: f64, v: f64, sign: f64) -> Option<Pt> {
        let (tu, tv) = (-e.gv, e.gu);
        let t = e.su * tu + e.sv * tv;
        let len = t.length();
        let sine = e.su.cross(e.sv).normalized()?.cross(e.n).length();
        (len > 0.0 && len.is_finite() && sine >= MIN_SINE).then(|| Pt {
            u,
            v,
            p: e.p,
            t: t * (sign / len),
            du: tu * sign / len,
            dv: tv * sign / len,
        })
    }

    // ----- cells and seeds -------------------------------------------------

    fn grid(&self) -> Grid {
        let b = self.chart.bounds;
        let (span_u, span_v) = (
            self.chart.u.1 - self.chart.u.0,
            self.chart.v.1 - self.chart.v.0,
        );
        let (wu, wv) = (b.lu * span_u, b.lv * span_v);
        let big = wu.max(wv);
        let base = |w: f64, periodic: bool| {
            let n = if big > 0.0 {
                (8.0 * w / big).ceil()
            } else {
                1.0
            };
            (n as i64).clamp(if periodic { 4 } else { 1 }, 8)
        };
        let (bu, bv) = (
            base(wu, self.chart.periodic_u),
            base(wv, self.chart.periodic_v),
        );
        let radius = 0.5 * (wu / bu as f64 + wv / bv as f64);
        let levels = if radius > self.tol.linear {
            ((radius / self.tol.linear).log2().ceil() as u32).min(MAX_LEVELS)
        } else {
            0
        };
        let base_size = 1_i64 << levels;
        let (nu, nv) = (bu * base_size, bv * base_size);
        Grid {
            u0: self.chart.u.0,
            v0: self.chart.v.0,
            du: span_u / nu as f64,
            dv: span_v / nv as f64,
            nu,
            nv,
            base: base_size,
        }
    }

    /// `f` at a grid node, remembered.
    fn corner(
        &mut self,
        grid: &Grid,
        cache: &mut BTreeMap<(i64, i64), f64>,
        i: i64,
        j: i64,
    ) -> f64 {
        let key = (
            if self.chart.periodic_u {
                i.rem_euclid(grid.nu)
            } else {
                i
            },
            if self.chart.periodic_v {
                j.rem_euclid(grid.nv)
            } else {
                j
            },
        );
        if let Some(f) = cache.get(&key) {
            return *f;
        }
        let (u, v) = grid.node(key.0, key.1);
        let f = self.eval(u, v).map_or(f64::NAN, |e| e.f);
        cache.insert(key, f);
        f
    }

    /// The zero of `f` on a grid edge whose ends have different signs.
    /// `along_u` edges run from `(a, fixed)` to `(b, fixed)`.
    fn edge_root(
        &mut self,
        grid: &Grid,
        along_u: bool,
        fixed: i64,
        (a, b): (i64, i64),
        (fa, fb): (f64, f64),
    ) -> Option<(f64, f64)> {
        let place = |s: f64| {
            if along_u {
                (grid.u0 + grid.du * s, grid.v0 + grid.dv * fixed as f64)
            } else {
                (grid.u0 + grid.du * fixed as f64, grid.v0 + grid.dv * s)
            }
        };
        let (mut lo, mut hi, mut flo, mut fhi) = (a as f64, b as f64, fa, fb);
        // Regula falsi with the Illinois correction; bisection as a net.
        let mut side = 0;
        for k in 0..80 {
            let s = if k % 4 == 3 {
                0.5 * (lo + hi)
            } else {
                (lo * fhi - hi * flo) / (fhi - flo)
            };
            if !(s > lo && s < hi) {
                break;
            }
            let (u, v) = place(s);
            let f = self.eval(u, v)?.f;
            if f.abs() <= self.on {
                return Some((u, v));
            }
            if (f >= 0.0) == (flo >= 0.0) {
                (lo, flo) = (s, f);
                if side == -1 {
                    fhi *= 0.5;
                }
                side = -1;
            } else {
                (hi, fhi) = (s, f);
                if side == 1 {
                    flo *= 0.5;
                }
                side = 1;
            }
        }
        Some(place(0.5 * (lo + hi)))
    }

    /// Seeds on an edge that lies on the border of the chart's rectangle,
    /// where a branch may come in and go out between two nodes.
    fn border_seeds(
        &mut self,
        grid: &Grid,
        cache: &mut BTreeMap<(i64, i64), f64>,
        along_u: bool,
        fixed: i64,
        span: (i64, i64),
        seeds: &mut Vec<(f64, f64)>,
    ) -> Result<(), GeomError> {
        let b = self.chart.bounds;
        let (speed, bend, step) = if along_u {
            (b.lu, b.muu, grid.du)
        } else {
            (b.lv, b.mvv, grid.dv)
        };
        let mut stack = vec![span];
        while let Some((lo, hi)) = stack.pop() {
            self.spent()?;
            let half = 0.5 * (hi - lo) as f64 * step;
            let mid = 0.5 * (lo + hi) as f64;
            let (u, v) = if along_u {
                (grid.u0 + grid.du * mid, grid.v0 + grid.dv * fixed as f64)
            } else {
                (grid.u0 + grid.du * fixed as f64, grid.v0 + grid.dv * mid)
            };
            let reach = speed * half;
            let monotonic = match self.eval(u, v) {
                Some(e) if e.dist > reach => continue,
                Some(e) => self.field.bend(e.p, reach).is_some_and(|k| {
                    let slope = if along_u { e.gu } else { e.gv };
                    slope.abs() > 1.01 * (k * speed * speed + bend) * half
                }),
                None => false,
            };
            if !monotonic && hi - lo > 1 {
                let middle = lo + (hi - lo) / 2;
                stack.push((middle, hi));
                stack.push((lo, middle));
                continue;
            }
            let node = |k: i64| if along_u { (k, fixed) } else { (fixed, k) };
            let (fa, fb) = (
                self.corner(grid, cache, node(lo).0, node(lo).1),
                self.corner(grid, cache, node(hi).0, node(hi).1),
            );
            if fa.is_finite() && fb.is_finite() && (fa >= 0.0) != (fb >= 0.0) {
                seeds.extend(self.edge_root(grid, along_u, fixed, (lo, hi), (fa, fb)));
            }
        }
        Ok(())
    }

    /// Divides the chart into cells; returns the seeds on the edges of the
    /// cells that may hold a piece of the intersection, and the centres of
    /// the floor cells.
    #[allow(clippy::type_complexity)]
    fn seeds(&mut self) -> Result<(Vec<(f64, f64)>, Vec<(f64, f64)>), GeomError> {
        let grid = self.grid();
        let b = self.chart.bounds;
        let mut stack: Vec<(i64, i64, i64)> = Vec::new();
        for i in (0..grid.nu / grid.base).rev() {
            for j in (0..grid.nv / grid.base).rev() {
                stack.push((i * grid.base, j * grid.base, grid.base));
            }
        }
        let mut leaves: Vec<(i64, i64, i64)> = Vec::new();
        let mut floor: Vec<(f64, f64)> = Vec::new();
        let mut cells = 0_usize;
        while let Some((i, j, size)) = stack.pop() {
            cells += 1;
            if cells > MAX_CELLS {
                return Err(too_many());
            }
            let (hu, hv) = (0.5 * size as f64 * grid.du, 0.5 * size as f64 * grid.dv);
            let (u, v) = (
                grid.u0 + grid.du * i as f64 + hu,
                grid.v0 + grid.dv * j as f64 + hv,
            );
            let reach = b.lu * hu + b.lv * hv;
            let simple = match self.eval(u, v) {
                Some(e) if e.dist > reach => continue,
                Some(e) => self.field.bend(e.p, reach).is_some_and(|k| {
                    let (muu, muv, mvv) = (
                        k * b.lu * b.lu + b.muu,
                        k * b.lu * b.lv + b.muv,
                        k * b.lv * b.lv + b.mvv,
                    );
                    e.gu.abs() > 1.01 * (muu * hu + muv * hv)
                        || e.gv.abs() > 1.01 * (muv * hu + mvv * hv)
                }),
                None => false,
            };
            if simple {
                leaves.push((i, j, size));
            } else if size == 1 {
                leaves.push((i, j, size));
                floor.push((u, v));
                if floor.len() > MAX_FLOOR_CELLS {
                    return Err(too_many());
                }
            } else {
                let h = size / 2;
                for (di, dj) in [(h, h), (0, h), (h, 0), (0, 0)] {
                    stack.push((i + di, j + dj, h));
                }
            }
        }
        // Every edge of every leaf, once.
        let mut edges: BTreeSet<(bool, i64, i64, i64)> = BTreeSet::new();
        for &(i, j, size) in &leaves {
            let wrap_u = |k: i64| {
                if self.chart.periodic_u {
                    k.rem_euclid(grid.nu)
                } else {
                    k
                }
            };
            let wrap_v = |k: i64| {
                if self.chart.periodic_v {
                    k.rem_euclid(grid.nv)
                } else {
                    k
                }
            };
            edges.insert((true, wrap_v(j), i, i + size));
            edges.insert((true, wrap_v(j + size), i, i + size));
            edges.insert((false, wrap_u(i), j, j + size));
            edges.insert((false, wrap_u(i + size), j, j + size));
        }
        let mut cache = BTreeMap::new();
        let mut seeds = Vec::new();
        for (along_u, fixed, lo, hi) in edges {
            self.spent()?;
            let border = if along_u {
                !self.chart.periodic_v && (fixed == 0 || fixed == grid.nv)
            } else {
                !self.chart.periodic_u && (fixed == 0 || fixed == grid.nu)
            };
            if border {
                self.border_seeds(&grid, &mut cache, along_u, fixed, (lo, hi), &mut seeds)?;
                continue;
            }
            let node = |k: i64| if along_u { (k, fixed) } else { (fixed, k) };
            let (fa, fb) = (
                self.corner(&grid, &mut cache, node(lo).0, node(lo).1),
                self.corner(&grid, &mut cache, node(hi).0, node(hi).1),
            );
            if fa.is_finite() && fb.is_finite() && (fa >= 0.0) != (fb >= 0.0) {
                seeds.extend(self.edge_root(&grid, along_u, fixed, (lo, hi), (fa, fb)));
            }
        }
        Ok((seeds, floor))
    }

    // ----- singular points ---------------------------------------------------

    /// The Hessian of `f` in parameters, by central differences of its
    /// gradient over about `1e-5` of the box.
    fn hessian(&mut self, u: f64, v: f64) -> Option<(f64, f64, f64)> {
        let b = self.chart.bounds;
        let reach = 3.2e-4 * self.hmax;
        let eu = if b.lu > 0.0 {
            reach / b.lu
        } else {
            1e-6 * (self.chart.u.1 - self.chart.u.0)
        };
        let ev = if b.lv > 0.0 {
            reach / b.lv
        } else {
            1e-6 * (self.chart.v.1 - self.chart.v.0)
        };
        let (a, b_, c, d) = (
            self.eval(u + eu, v)?,
            self.eval(u - eu, v)?,
            self.eval(u, v + ev)?,
            self.eval(u, v - ev)?,
        );
        let fuu = (a.gu - b_.gu) / (2.0 * eu);
        let fvv = (c.gv - d.gv) / (2.0 * ev);
        let fuv = 0.5 * ((a.gv - b_.gv) / (2.0 * eu) + (c.gu - d.gu) / (2.0 * ev));
        (fuu.is_finite() && fuv.is_finite() && fvv.is_finite()).then_some((fuu, fuv, fvv))
    }

    /// Newton on the gradient of `f` from a floor cell: the point where the
    /// surfaces are tangent, if it is one where they also touch.
    fn critical(&mut self, u0: f64, v0: f64) -> Result<Option<Singular>, GeomError> {
        let (mut u, mut v) = (u0, v0);
        let Some(start) = self.eval(u0, v0) else {
            return Ok(None);
        };
        for _ in 0..30 {
            let (Some(e), Some((a, b, c))) = (self.eval(u, v), self.hessian(u, v)) else {
                return Ok(None);
            };
            let (lu, lv) = (e.su.length(), e.sv.length());
            if !(lu > 0.0 && lv > 0.0) {
                return Ok(None);
            }
            // In units of length: the Hessian of f along the surface.
            let (ha, hb, hc) = (a / (lu * lu), b / (lu * lv), c / (lv * lv));
            let det = ha * hc - hb * hb;
            let size = ha.abs().max(hb.abs()).max(hc.abs());
            let flat = !(det.abs() > 1e-3 * size * size);
            let (gu, gv) = (e.gu / lu, e.gv / lv);
            let (step_u, step_v) = if flat {
                (0.0, 0.0)
            } else {
                ((hb * gv - hc * gu) / det, (hb * gu - ha * gv) / det)
            };
            let moved = step_u.hypot(step_v);
            if flat || moved <= self.on {
                // Converged (or no better step exists).
                let tangent = gu.hypot(gv) <= 1e-6_f64.max(4.0 * size * self.tol.linear);
                if !(tangent && e.dist <= self.tol.linear && self.inside(u, v)) {
                    return Ok(None);
                }
                if flat {
                    return Err(GeomError::Unsupported(
                        "surfaces that touch along a curve, or with more than second-order contact, without a closed form",
                    ));
                }
                return Ok(Some(Singular {
                    u,
                    v,
                    p: e.p,
                    kind: if det > 0.0 {
                        SingularKind::Touch
                    } else {
                        SingularKind::Cross
                    },
                }));
            }
            u += step_u / lu;
            v += step_v / lv;
            if !(u.is_finite() && v.is_finite()) || e.p.distance(start.p) > 64.0 * self.hmax {
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Seeds on the four arms that leave a crossing point: along the
    /// directions in which the Hessian vanishes.
    fn arms(&mut self, s: &Singular, seeds: &mut Vec<(f64, f64)>) {
        let (Some(e), Some((a, b, c))) = (self.eval(s.u, s.v), self.hessian(s.u, s.v)) else {
            return;
        };
        let root = (b * b - a * c).max(0.0).sqrt();
        let directions = if a.abs() >= c.abs() && a != 0.0 {
            [((-b + root) / a, 1.0), ((-b - root) / a, 1.0)]
        } else if c != 0.0 {
            [(1.0, (-b + root) / c), (1.0, (-b - root) / c)]
        } else {
            [(1.0, 0.0), (0.0, 1.0)]
        };
        let radius = (64.0 * self.tol.linear).max(1e-2 * self.hmax);
        for (du, dv) in directions {
            let len = (e.su * du + e.sv * dv).length();
            if !(len > 0.0) {
                continue;
            }
            for side in [1.0, -1.0] {
                let k = side * radius / len;
                if let Some((_, u, v)) = self.correct(s.u + du * k, s.v + dv * k) {
                    seeds.push((u, v));
                }
            }
        }
    }

    // ----- marching ------------------------------------------------------------

    /// `x` moved by whole periods to be nearest to `near`.
    fn unwrap(x: f64, near: f64, periodic: bool, period: f64) -> f64 {
        if periodic {
            x + ((near - x) / period).round() * period
        } else {
            x
        }
    }

    fn near(&self, (u, v): (f64, f64), to: &Pt) -> (f64, f64) {
        let c = &self.chart;
        (
            Self::unwrap(u, to.u, c.periodic_u, c.u.1 - c.u.0),
            Self::unwrap(v, to.v, c.periodic_v, c.v.1 - c.v.0),
        )
    }

    /// Where the step from `cur` to `(u, v)` leaves the chart's rectangle.
    fn exit(&mut self, cur: &Pt, u: f64, v: f64, sign: f64) -> Option<Pt> {
        let c = &self.chart;
        let mut bounds: Vec<(f64, bool, f64)> = Vec::new(); // (fraction, on u, bound)
        if !c.periodic_u {
            for bound in [c.u.0, c.u.1] {
                if (u - bound) * (cur.u - bound) < 0.0 {
                    bounds.push(((bound - cur.u) / (u - cur.u), true, bound));
                }
            }
        }
        if !c.periodic_v {
            for bound in [c.v.0, c.v.1] {
                if (v - bound) * (cur.v - bound) < 0.0 {
                    bounds.push(((bound - cur.v) / (v - cur.v), false, bound));
                }
            }
        }
        bounds.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (range_u, range_v) = (c.u, c.v);
        for (fraction, on_u, bound) in bounds {
            // Newton along the border from the straight guess.
            let mut y = if on_u {
                cur.v + fraction * (v - cur.v)
            } else {
                cur.u + fraction * (u - cur.u)
            };
            for _ in 0..24 {
                let (pu, pv) = if on_u { (bound, y) } else { (y, bound) };
                let Some(e) = self.eval(pu, pv) else { break };
                if e.f.abs() <= self.on {
                    let slack_u = 1e-9 * (range_u.1 - range_u.0);
                    let slack_v = 1e-9 * (range_v.1 - range_v.0);
                    let ok = if on_u {
                        y >= range_v.0 - slack_v && y <= range_v.1 + slack_v
                    } else {
                        y >= range_u.0 - slack_u && y <= range_u.1 + slack_u
                    };
                    if !ok || e.dist > self.cut {
                        break;
                    }
                    let chord = (e.p - cur.p).normalized();
                    return Self::point(&e, pu, pv, sign).or_else(|| {
                        chord.map(|t| Pt {
                            u: pu,
                            v: pv,
                            p: e.p,
                            t,
                            du: 0.0,
                            dv: 0.0,
                        })
                    });
                }
                let slope = if on_u { e.gv } else { e.gu };
                if slope == 0.0 {
                    break;
                }
                y -= e.f / slope;
                if !y.is_finite() {
                    break;
                }
            }
        }
        None
    }

    /// Follows a branch from `start` in the direction of its tangent.
    fn run(
        &mut self,
        start: &Pt,
        sign: f64,
        singular: &[Singular],
        may_close: bool,
    ) -> Result<(Vec<Pt>, End), GeomError> {
        let mut pts: Vec<Pt> = Vec::new();
        let mut cur = *start;
        let mut h = 0.25 * self.hmax;
        let cos_turn = MAX_TURN.cos();
        let cos_half = (0.5 * MAX_TURN).cos();
        let hmin = self.hmin;
        loop {
            self.spent()?;
            if pts.len() > MAX_POINTS {
                return Err(GeomError::NoConvergence(
                    "an intersection curve needs more points than the cap allows",
                ));
            }
            // A crossing point or an apex straight ahead ends the branch.
            let mut snapped = None;
            for s in singular {
                if s.kind == SingularKind::Touch {
                    continue;
                }
                let d = s.p - cur.p;
                let dist = d.length();
                let along = d.dot(cur.t);
                if !(dist > self.on && dist <= h && along > 0.0) {
                    continue;
                }
                if (d - cur.t * along).length() > 0.1 * dist {
                    continue;
                }
                // The curve really goes there: its middle is near the chord.
                let (su, sv) = self.near((s.u, s.v), &cur);
                let Some((m, _, _)) = self.correct(0.5 * (cur.u + su), 0.5 * (cur.v + sv)) else {
                    continue;
                };
                let off = m.p - cur.p;
                let t = off.dot(d) / (dist * dist);
                if t > 0.0 && t < 1.0 && (off - d * t).length() <= 0.2 * dist {
                    snapped = Some(Pt {
                        u: su,
                        v: sv,
                        p: s.p,
                        t: d * (1.0 / dist),
                        du: 0.0,
                        dv: 0.0,
                    });
                    break;
                }
            }
            if let Some(end) = snapped {
                pts.push(end);
                return Ok((pts, End::Open));
            }
            let (pu, pv) = (cur.u + h * cur.du, cur.v + h * cur.dv);
            let Some((e, u, v)) = self.correct(pu, pv) else {
                // Past the border of the chart there may be nothing to
                // correct onto: end on the border if the curve reaches it.
                if !self.inside(pu, pv)
                    && let Some(end) = self.exit(&cur, pu, pv, sign)
                {
                    pts.push(end);
                    return Ok((pts, End::Open));
                }
                h = halved(h, hmin)?;
                continue;
            };
            let step = e.p - cur.p;
            let len = step.length();
            let new = Self::point(&e, u, v, sign);
            let turn = new.map_or(-1.0, |n| n.t.dot(cur.t));
            let Some(new) =
                new.filter(|_| step.dot(cur.t) > 0.0 && len <= 2.0 * h && turn >= cos_turn)
            else {
                // Off the edge of the other surface the field has a crease
                // and no tangent: that is an end, not a stall.
                if e.dist > self.cut {
                    pts.extend(self.edge(&cur, h, sign));
                    return Ok((pts, End::Open));
                }
                h = halved(h, hmin)?;
                continue;
            };
            if may_close && pts.len() >= 2 {
                let back = start.p - cur.p;
                let s = back.dot(cur.t);
                if s > 0.0
                    && s <= len * (1.0 + 1e-9)
                    && (back - cur.t * s).length() <= 0.2 * len + self.on
                    && start.t.dot(cur.t) > 0.5
                {
                    return Ok((pts, End::Closed));
                }
            }
            if e.dist > self.cut {
                pts.extend(self.edge(&cur, h, sign));
                return Ok((pts, End::Open));
            }
            if !self.inside(u, v) {
                // The border is closer than this step: land on it, or come
                // closer and try again.
                if let Some(end) = self.exit(&cur, u, v, sign) {
                    pts.push(end);
                    return Ok((pts, End::Open));
                }
                h = halved(h, hmin)?;
                continue;
            }
            pts.push(new);
            cur = new;
            if turn >= cos_half {
                h = (1.5 * h).min(self.hmax);
            }
        }
    }

    /// The last point before the edge of the field's surface, on the step
    /// of length `h` from `cur`: by bisection on the step.
    fn edge(&mut self, cur: &Pt, h: f64, sign: f64) -> Option<Pt> {
        let (mut lo, mut hi) = (0.0, 1.0);
        let mut best = None;
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            let found = self
                .correct(cur.u + mid * h * cur.du, cur.v + mid * h * cur.dv)
                .filter(|(e, u, v)| e.dist <= self.cut && self.inside(*u, *v))
                .and_then(|(e, u, v)| {
                    let chord = (e.p - cur.p).normalized()?;
                    Some(Self::point(&e, u, v, sign).unwrap_or(Pt {
                        u,
                        v,
                        p: e.p,
                        t: chord,
                        du: 0.0,
                        dv: 0.0,
                    }))
                })
                .filter(|pt| (pt.p - cur.p).dot(cur.t) > 0.0);
            match found {
                Some(pt) => {
                    best = Some(pt);
                    lo = mid;
                }
                None => hi = mid,
            }
            if (hi - lo) * h <= self.on {
                break;
            }
        }
        best.filter(|pt| pt.p.distance(cur.p) > self.on)
    }

    fn trace(
        &mut self,
        seed: (f64, f64),
        singular: &[Singular],
    ) -> Result<Option<Branch>, GeomError> {
        let Some((e, u, v)) = self.correct(seed.0, seed.1) else {
            return Ok(None);
        };
        if e.dist > self.cut || !self.inside(u, v) {
            return Ok(None);
        }
        let Some(start) = Self::point(&e, u, v, 1.0) else {
            return Ok(None);
        };
        let (forward, end) = self.run(&start, 1.0, singular, true)?;
        let mut pts;
        let closed = end == End::Closed;
        if closed {
            pts = vec![start];
            pts.extend(forward);
            // Close on the start, with parameters that continue the last.
            if let Some(last) = pts.last().copied() {
                let (cu, cv) = self.near((start.u, start.v), &last);
                pts.push(Pt {
                    u: cu,
                    v: cv,
                    ..start
                });
            }
        } else {
            let (backward, _) = self.run(&start.reversed(), -1.0, singular, false)?;
            pts = backward.into_iter().rev().map(Pt::reversed).collect();
            pts.push(start);
            pts.extend(forward);
        }
        if pts.len() < 2 {
            return Ok(None);
        }
        self.refine(&mut pts)?;
        Ok(Some(Branch { pts, closed }))
    }

    // ----- fitting ---------------------------------------------------------------

    /// The handle length of the cubic span from `a` to `b` as a fraction of
    /// the chord: the value that makes a circular arc of the same turn
    /// exact to sixth order, `(2/3) tan(θ/4) / sin(θ/2)`.
    fn handle(a: &Pt, b: &Pt) -> f64 {
        let theta = a.t.dot(b.t).clamp(-1.0, 1.0).acos().min(2.0);
        if theta < 1e-4 {
            1.0 / 3.0
        } else {
            (2.0 / 3.0) * (0.25 * theta).tan() / (0.5 * theta).sin()
        }
    }

    /// The four Bézier points of the span from `a` to `b`.
    fn span(a: &Pt, b: &Pt) -> [Vec3; 4] {
        let l = a.p.distance(b.p) * Self::handle(a, b);
        [a.p, a.p + a.t * l, b.p - b.t * l, b.p]
    }

    fn bezier(c: &[Vec3; 4], s: f64) -> Vec3 {
        let r = 1.0 - s;
        let [c0, c1, c2, c3] = *c;
        c0 * (r * r * r) + c1 * (3.0 * r * r * s) + c2 * (3.0 * r * s * s) + c3 * (s * s * s)
    }

    /// How far the span from `a` to `b` is from either surface, at three
    /// places.
    fn deviation(&mut self, a: &Pt, b: &Pt) -> f64 {
        let c = Self::span(a, b);
        let mut worst = 0.0_f64;
        for s in [0.25, 0.5, 0.75] {
            let q = Self::bezier(&c, s);
            let other = self.field.sample(q).map_or(f64::INFINITY, |x| x.dist);
            let own = match &self.own {
                Field::Free(projector) => {
                    let (_, _, foot) =
                        projector.foot_from(q, a.u + (b.u - a.u) * s, a.v + (b.v - a.v) * s);
                    foot.distance(q)
                }
                field => field.sample(q).map_or(f64::INFINITY, |x| x.dist),
            };
            self.evals += 2;
            worst = worst.max(other).max(own);
        }
        worst
    }

    /// A point of the branch between `a` and `b`.
    fn between(&mut self, a: &Pt, b: &Pt) -> Option<Pt> {
        let (e, u, v) = self.correct(0.5 * (a.u + b.u), 0.5 * (a.v + b.v))?;
        let chord = b.p - a.p;
        let along = (e.p - a.p).dot(chord);
        if !(along > 0.0 && along < chord.length_squared()) {
            return None;
        }
        let sign = if (-e.gv * e.su.dot(chord) + e.gu * e.sv.dot(chord)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        Self::point(&e, u, v, sign)
    }

    /// Adds points until every span is within half the tolerance of both
    /// surfaces at its sampled places.
    fn refine(&mut self, pts: &mut Vec<Pt>) -> Result<(), GeomError> {
        let target = 0.5 * self.tol.linear;
        let mut i = 0;
        while let (Some(a), Some(b)) = (pts.get(i).copied(), pts.get(i + 1).copied()) {
            self.spent()?;
            if self.deviation(&a, &b) <= target {
                i += 1;
                continue;
            }
            let give_up = GeomError::NoConvergence(
                "an intersection curve could not be fitted within the tolerance",
            );
            if pts.len() >= 2 * MAX_POINTS || a.p.distance(b.p) <= 8.0 * self.on {
                return Err(give_up);
            }
            let middle = self.between(&a, &b).ok_or(give_up)?;
            pts.insert(i + 1, middle);
        }
        Ok(())
    }

    /// True when `p` lies on the branch.
    fn on_branch(&self, p: Vec3, pts: &[Pt]) -> bool {
        let slack = 100.0 * self.on;
        for (k, end) in [pts.first(), pts.last()].into_iter().flatten().enumerate() {
            // A seed just past an end is the same branch.
            let ahead = (p - end.p).dot(end.t) * if k == 0 { -1.0 } else { 1.0 };
            if p.distance(end.p) <= 2.0 * self.cut && ahead >= 0.0 {
                return true;
            }
        }
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = b.p - a.p;
            let len2 = chord.length_squared();
            if !(len2 > 0.0) {
                continue;
            }
            let len = len2.sqrt();
            let t = ((p - a.p).dot(chord) / len2).clamp(0.0, 1.0);
            if (a.p + chord * t).distance(p) > 0.1 * len + slack {
                continue;
            }
            let c = Self::span(a, b);
            let mut prev = a.p;
            for k in 1..=8 {
                let q = Self::bezier(&c, f64::from(k) / 8.0);
                let seg = q - prev;
                let l2 = seg.length_squared();
                let s = if l2 > 0.0 {
                    ((p - prev).dot(seg) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if (prev + seg * s).distance(p) <= 2e-3 * len + slack {
                    return true;
                }
                prev = q;
            }
        }
        false
    }

    /// The C¹ cubic through the points, and how far it is from the
    /// surfaces where that was measured.
    fn fit(&mut self, pts: &[Pt]) -> Result<(NurbsCurve, f64, f64), GeomError> {
        let mut knots = vec![0.0; 4];
        let mut ctrl = Vec::with_capacity(2 * pts.len());
        let mut total = 0.0;
        let mut worst = 0.0_f64;
        let mut spans = 0;
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = a.p.distance(b.p);
            if !(chord > 0.0) {
                continue;
            }
            let c = Self::span(a, b);
            if spans == 0 {
                ctrl.push(a.p);
            } else {
                knots.push(total);
                knots.push(total);
            }
            // A knot span of three handles makes the derivative the unit
            // tangent on both sides of every joint.
            total += 3.0 * chord * Self::handle(a, b);
            let [_, c1, c2, _] = c;
            ctrl.push(c1);
            ctrl.push(c2);
            spans += 1;
            worst = worst.max(self.deviation(a, b));
        }
        let last = pts
            .last()
            .ok_or(GeomError::InvalidParameter("an empty intersection curve"))?;
        ctrl.push(last.p);
        knots.extend([total; 4]);
        if spans == 0 {
            return Err(GeomError::InvalidParameter("an empty intersection curve"));
        }
        Ok((NurbsCurve::polynomial(3, knots, ctrl)?, total, worst))
    }

    // ----- clipping to the box --------------------------------------------------

    /// The last point inside the box on the way from `a` (inside) to `b`
    /// (outside).
    fn box_crossing(&mut self, a: &Pt, b: &Pt, within: &Bbox3, flip: bool) -> Option<Pt> {
        let (mut lo, mut hi) = (0.0, 1.0);
        let mut best = None;
        let chord = b.p - a.p;
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            let found = self
                .correct(a.u + (b.u - a.u) * mid, a.v + (b.v - a.v) * mid)
                .filter(|(e, _, _)| within.contains(e.p, 0.0));
            match found {
                Some((e, u, v)) => {
                    let forward = -e.gv * e.su.dot(chord) + e.gu * e.sv.dot(chord) >= 0.0;
                    let sign = if forward != flip { 1.0 } else { -1.0 };
                    best = Self::point(&e, u, v, sign).or(best);
                    lo = mid;
                }
                None => hi = mid,
            }
            if (hi - lo) * chord.length() <= self.on {
                break;
            }
        }
        best
    }

    /// The pieces of a branch inside the box.
    fn clip(&mut self, branch: &Branch, within: &Bbox3) -> Vec<(Vec<Pt>, bool)> {
        let pts = &branch.pts;
        let inside: Vec<bool> = pts.iter().map(|p| within.contains(p.p, 0.0)).collect();
        if inside.iter().all(|x| *x) {
            return vec![(pts.clone(), branch.closed)];
        }
        // Start a closed branch at a point outside, so no piece spans the seam.
        let order: Vec<usize> = match (branch.closed, inside.iter().position(|x| !*x)) {
            (true, Some(first)) => {
                let n = pts.len().saturating_sub(1).max(1);
                (0..=n).map(|k| (first + k) % n).collect()
            }
            _ => (0..pts.len()).collect(),
        };
        let mut pieces = Vec::new();
        let mut piece: Vec<Pt> = Vec::new();
        let mut prev: Option<(Pt, bool)> = None;
        let mut shift = (0.0, 0.0);
        for k in order {
            let (Some(raw), Some(&is_in)) = (pts.get(k), inside.get(k)) else {
                continue;
            };
            // Keep parameters continuous across the seam of a closed branch.
            let mut pt = *raw;
            if let Some((before, _)) = prev {
                let (u, v) = self.near((pt.u + shift.0, pt.v + shift.1), &before);
                shift = (u - raw.u, v - raw.v);
                (pt.u, pt.v) = (u, v);
            }
            if let Some((before, was_in)) = prev {
                if was_in && !is_in {
                    piece.extend(self.box_crossing(&before, &pt, within, false));
                    pieces.push(std::mem::take(&mut piece));
                } else if !was_in && is_in {
                    piece.extend(self.box_crossing(&pt, &before, within, true));
                }
            }
            if is_in {
                piece.push(pt);
            }
            prev = Some((pt, is_in));
        }
        pieces.push(piece);
        pieces
            .into_iter()
            .filter(|p| p.len() >= 2)
            .map(|p| (p, false))
            .collect()
    }

    // ----- the whole intersection -----------------------------------------------

    pub(super) fn solve(&mut self, within: &Bbox3) -> Result<Meeting, GeomError> {
        let (mut seeds, floor) = self.seeds()?;
        let mut singular: Vec<Singular> = Vec::new();
        // The apex of a cone the chart passes through.
        if let Some(apex) = self.field.apex()
            && let Ok((u, v)) = self.chart.surface.project(apex)
            && self.chart.surface.eval(u, v).distance(apex) <= self.tol.linear
        {
            singular.push(Singular {
                u,
                v,
                p: apex,
                kind: SingularKind::Apex,
            });
        }
        let merge = 8.0 * self.tol.linear;
        for (u, v) in floor {
            self.spent()?;
            if singular.iter().any(|s| {
                s.kind == SingularKind::Apex
                    && self.chart.surface.eval(u, v).distance(s.p) <= 2.0 * merge
            }) {
                continue;
            }
            if let Some(found) = self.critical(u, v)?
                && !singular.iter().any(|s| s.p.distance(found.p) <= merge)
            {
                singular.push(found);
            }
        }
        for s in singular.clone() {
            if s.kind == SingularKind::Cross {
                self.arms(&s, &mut seeds);
            }
        }
        let mut branches: Vec<Branch> = Vec::new();
        for seed in seeds {
            self.spent()?;
            let p = self.chart.surface.eval(seed.0, seed.1);
            if branches.iter().any(|b| self.on_branch(p, &b.pts))
                || singular.iter().any(|s| p.distance(s.p) <= 2.0 * merge)
            {
                continue;
            }
            if let Some(branch) = self.trace(seed, &singular)? {
                if branches.len() >= MAX_BRANCHES {
                    return Err(GeomError::NoConvergence(
                        "the intersection has more branches than the cap allows",
                    ));
                }
                branches.push(branch);
            }
        }
        let mut curves = Vec::new();
        for branch in &branches {
            for (pts, closed) in self.clip(branch, within) {
                let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
                    continue;
                };
                let length: f64 = pts
                    .iter()
                    .zip(pts.iter().skip(1))
                    .map(|(a, b)| a.p.distance(b.p))
                    .sum();
                if !closed
                    && first.p.distance(last.p) <= self.tol.linear
                    && length <= self.tol.linear
                {
                    continue; // Too short to be a curve.
                }
                let (curve, total, worst) = self.fit(&pts)?;
                let deviation = 1.5 * worst + 4.0 * self.on;
                if !(deviation <= self.tol.linear) {
                    return Err(GeomError::NoConvergence(
                        "an intersection curve could not be fitted within the tolerance",
                    ));
                }
                curves.push(MeetingCurve {
                    curve: Curve3::Nurbs(curve),
                    range: (0.0, total),
                    closed,
                    tangent: false,
                    exact: false,
                    deviation,
                });
            }
        }
        let points = singular
            .iter()
            .filter(|s| s.kind == SingularKind::Touch && within.contains(s.p, self.tol.linear))
            .map(|s| s.p)
            .collect();
        Ok(Meeting { curves, points })
    }

    /// True when every one of a grid of points of `a` lies on `b`; the
    /// second value says whether their normals agree there.
    pub(super) fn lies_on(
        a: &Surface,
        field: &Field,
        within: &Bbox3,
        tol: &Tolerance,
    ) -> Option<bool> {
        let chart = Chart::new(a, within, tol, field).ok()??;
        let mut same = None;
        for i in 0..9 {
            for j in 0..9 {
                let u = chart.u.0 + (chart.u.1 - chart.u.0) * (f64::from(i) + 0.5) / 9.0;
                let v = chart.v.0 + (chart.v.1 - chart.v.0) * (f64::from(j) + 0.5) / 9.0;
                let sample = field.sample(chart.surface.eval(u, v))?;
                if sample.dist > tol.linear {
                    return None;
                }
                if same.is_none()
                    && let Some(n) = chart.surface.normal(u, v)
                {
                    same = Some(n.dot(sample.normal) > 0.0);
                }
            }
        }
        same
    }
}
