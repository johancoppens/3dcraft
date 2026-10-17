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
//! around it where the surfaces go through each other by no more than the
//! linear tolerance is indistinguishable from it), a saddle is a point
//! where two branches cross (every arm gets a seed, and the march stops
//! there). Where the Hessian has rank one the point is looked for along the
//! valley of its strong direction, and its arms (a tacnode's four, a
//! cusp's two) are made as graphs in a disc around it (`tacnode.rs`); a
//! contact that keeps to one side is a touch; anything else is refused.
//!
//! **Discs around critical points.** Around a critical point at distance
//! `r` the gradient of `f` is about `λ r` (`λ` the smaller eigenvalue of
//! the Hessian), and a cell of size `h` is simple only where that exceeds
//! `M h`: at every level about `π (M/λ)²` cells around the point are
//! neither empty nor simple, thousands when the bound `M` is loose or the
//! contact nearly degenerate, down to the floor. So the critical point is
//! looked for as soon as cells keep failing both tests (and from every
//! floor cell), and where one is found whose second-order model
//! `∇f(x) ≈ H (x − c)` holds within a quarter on two rings around it, the
//! disc inside the outer ring is settled without cells: `f` has no other
//! critical point there, so its zero set in the disc is the point itself
//! (a touch), the two branches through it (a crossing, seeded by its
//! arms), or nothing (a critical point off the other surface with `f` of
//! the sign its Hessian keeps). The rings are samples, not a proof: the
//! same standing as the sampled derivative bounds below.
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

mod tacnode;

use super::field::{Chart, Cusp, Field, Sample, TOUCH};
use super::meet::{Meeting, MeetingCurve};
use crate::{Curve3, GeomError, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};

/// Cells looked at before the surfaces are declared too close to tangent
/// (or coincident) over too large an area. A flat face against a domed
/// roof at a point where it is tangent stays near the roof over much of
/// it, and the cells there are simple only when small: two such pairs of
/// the corpus (tangent/block-spline/005, tangent/cylinder-spline/008) need
/// 401,264 and 440,000, in about 1.6 s, and their common volumes agree
/// with the volume integrated directly to 1e-7. (400,000 until
/// 2026-10-14.)
const MAX_CELLS: usize = 1_000_000;
const MAX_FLOOR_CELLS: usize = 2_000;
const MAX_POINTS: usize = 16_000;
const MAX_BRANCHES: usize = 512;
const MAX_EVALS: usize = 40_000_000;
/// Cells that are neither empty nor simple before a critical point is
/// looked for the first time; doubled after every look that settles
/// nothing.
const FIRST_LOOK: usize = 16;
/// How far the gradient of `f` may stray from its second-order model on
/// the rings of a disc, as a fraction of the model.
const DISC_MODEL: f64 = 0.25;
/// Deepest subdivision: the finest cell is `2^-MAX_LEVELS` of a base cell.
const MAX_LEVELS: u32 = 40;
/// Largest turn of the tangent in one step, in radians.
const MAX_TURN: f64 = 0.2;
/// Sine of the angle between the surfaces below which a point is not
/// marched through.
const MIN_SINE: f64 = 1e-8;
/// How often a step that goes too far or too short, however short it is
/// made, is scaled anew ([`Marcher::rescaled`]): halving between a scale
/// too short and one too far closes in on the window of a step (half to
/// twice its length) across a jump in speed of `2^(RESCALES − 2)` and
/// more.
const RESCALES: usize = 48;

fn stalled() -> GeomError {
    GeomError::NoConvergence(
        "marching an intersection curve stalled (the surfaces are nearly tangent along it, or one of them has a crease)",
    )
}

/// Half the step, or the error that marching has stalled.
///
/// (Where the march cannot go on outside the box the intersection is asked
/// in, the branch ends there: nothing is asked there, the box cuts it
/// anyway, and the cells there were not looked at, so a point of tangency
/// or a crossing the march meets there is not known. The engine block's
/// fillet tool and rib, two cylinders that touch just outside the box of
/// their faces, stalled at the touch.)
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
    /// A point of contact of higher order with arms that leave it in the
    /// same direction (a tacnode or a cusp, `tacnode.rs`).
    Tacnode,
}

#[derive(Clone, Copy, Debug)]
struct Singular {
    u: f64,
    v: f64,
    p: Vec3,
    kind: SingularKind,
    /// How far from a crossing point its arms are seeded.
    arm: f64,
    /// How far from a crossing point the branches of `f = 0` turn away
    /// from it, where the surfaces are a little apart there.
    turn: f64,
}

/// Where Newton on the gradient of `f` came to rest.
#[derive(Clone, Copy, Debug)]
struct Stationary {
    u: f64,
    v: f64,
    e: Eval,
    /// The lengths of the chart's derivatives there.
    lu: f64,
    lv: f64,
    /// The gradient and the Hessian of `f` along the surface, in units of
    /// length.
    g: (f64, f64),
    h: (f64, f64, f64),
    /// No step was left to take: the Hessian is degenerate there.
    flat: bool,
}

/// An ellipse in the chart's parameters around a critical point,
/// `((u − cu) lu)² + ((v − cv) lv)² ≤ radius²`, inside which the zero set
/// of `f` is known without cells.
#[derive(Clone, Copy, Debug)]
struct Disc {
    u: f64,
    v: f64,
    lu: f64,
    lv: f64,
    radius: f64,
}

impl Disc {
    fn holds(&self, u: f64, v: f64) -> bool {
        ((u - self.u) * self.lu).hypot((v - self.v) * self.lv) <= self.radius
    }

    /// True when the cell around `(u, v)` of half sizes `(hu, hv)` lies in
    /// the disc: an ellipse is convex, so its corners decide.
    fn covers(&self, u: f64, v: f64, hu: f64, hv: f64) -> bool {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
            .iter()
            .all(|&(su, sv)| self.holds(u + su * hu, v + sv * hv))
    }
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
    /// Borders of the chart's rectangle (u start, u end, v start, v end)
    /// that lie on the field's surface: reported as curves, not marched.
    lying: [bool; 4],
    /// Crossing points read as such although the surfaces are a little
    /// apart there, each with the radius within which a branch is drawn
    /// straight to the point instead of along `f = 0`, which turns away
    /// before it gets there.
    zones: Vec<(Vec3, f64)>,
    /// Points of contact of higher order whose arms are made near them.
    tacnodes: Vec<tacnode::Tacnode>,
    /// The chart of the field's surface when that is freeform: its
    /// borders are edges, where the field goes on past the surface.
    field_chart: Option<Chart>,
    /// The box the intersection is asked in.
    within: Bbox3,
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
            lying: [false; 4],
            zones: Vec::new(),
            tacnodes: Vec::new(),
            field_chart: None,
            within: *within,
        })
    }

    /// Tells the marcher the chart of the field's surface, when that is a
    /// freeform surface with borders (see [`Marcher::field_border_seeds`]).
    pub(super) fn with_field_chart(mut self, chart: Chart) -> Self {
        self.field_chart = Some(chart);
        self
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
        let (p, su, sv) = self.chart.at(u, v);
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

    /// True when a point is on the border of the chart's rectangle.
    fn on_border(&self, p: &Pt) -> bool {
        let c = &self.chart;
        let at = |x: f64, (lo, hi): (f64, f64)| {
            let slack = 1e-12 * (hi - lo);
            x <= lo + slack || x >= hi - slack
        };
        (!c.periodic_u && at(p.u, c.u)) || (!c.periodic_v && at(p.v, c.v))
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
            let s = if s > lo && s < hi { s } else { 0.5 * (lo + hi) };
            if !(s > lo && s < hi) {
                break; // The bracket is down to rounding.
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

    /// The borders of a freeform chart that lie on the field's surface,
    /// as curves. The intersection then runs along the edge of the
    /// rectangle, where no cell edge shows a change of sign and every cell
    /// along it would be split to the floor for nothing (a plane through
    /// the edge of a patch, which is every patch standing on a plate). The
    /// border is the surface's own iso-curve there, exact on the chart and
    /// on the field to the sampled deviation.
    fn border_curves(&mut self) -> Result<Vec<MeetingCurve>, GeomError> {
        let spline = match &self.chart.surface {
            Surface::Bezier(b) => NurbsSurface::from_bezier(b)?,
            Surface::Nurbs(s) => s.clone(),
            _ => return Ok(Vec::new()),
        };
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let (degree_u, degree_v) = spline.degrees();
        let (count_u, count_v) = spline.counts();
        let (knots_u, knots_v) = (spline.knots_u(), spline.knots_v());
        let (ctrl, weights) = (spline.control_points(), spline.weights());
        // A border is the iso-curve of its row of the net only where the
        // knots are clamped at that end.
        let clamped = |knots: &[f64], degree: usize, count: usize, at_end: bool| {
            if at_end {
                knots.get(count) == knots.get(count + degree)
            } else {
                knots.first() == knots.get(degree)
            }
        };
        const SAMPLES: u32 = 64;
        let mut curves = Vec::new();
        for side in 0..4 {
            let along_u = side >= 2;
            let at_end = side % 2 == 1;
            // A surface that closes on itself has no border where it goes
            // around; a border across that way (the rim of a closed wall)
            // runs around and closes on itself.
            if (along_u && self.chart.periodic_v) || (!along_u && self.chart.periodic_u) {
                continue;
            }
            let around = if along_u {
                self.chart.periodic_u
            } else {
                self.chart.periodic_v
            };
            let fixed = match side {
                0 => u0,
                1 => u1,
                2 => v0,
                _ => v1,
            };
            let mut worst = 0.0_f64;
            let mut on_field = true;
            for k in 0..=SAMPLES {
                self.spent()?;
                let t = f64::from(k) / f64::from(SAMPLES);
                let (u, v) = if along_u {
                    (u0 + (u1 - u0) * t, fixed)
                } else {
                    (fixed, v0 + (v1 - v0) * t)
                };
                match self.eval(u, v) {
                    Some(e) if e.dist <= self.tol.linear => worst = worst.max(e.dist),
                    _ => {
                        on_field = false;
                        break;
                    }
                }
            }
            if !on_field {
                continue;
            }
            let (points, curve_weights, knots, degree): (Vec<Vec3>, Vec<f64>, Vec<f64>, usize) =
                if along_u {
                    if !clamped(knots_v, degree_v, count_v, at_end) {
                        continue;
                    }
                    let j = if at_end { count_v - 1 } else { 0 };
                    let row = |i: usize| i * count_v + j;
                    (
                        (0..count_u)
                            .filter_map(|i| ctrl.get(row(i)).copied())
                            .collect(),
                        (0..count_u)
                            .filter_map(|i| weights.get(row(i)).copied())
                            .collect(),
                        knots_u.to_vec(),
                        degree_u,
                    )
                } else {
                    if !clamped(knots_u, degree_u, count_u, at_end) {
                        continue;
                    }
                    let i = if at_end { count_u - 1 } else { 0 };
                    (
                        (0..count_v)
                            .filter_map(|j| ctrl.get(i * count_v + j).copied())
                            .collect(),
                        (0..count_v)
                            .filter_map(|j| weights.get(i * count_v + j).copied())
                            .collect(),
                        knots_v.to_vec(),
                        degree_v,
                    )
                };
            // A border that is one point (the pole of a ball, or of a
            // dome or an octant on an open patch) lies on the field when
            // the pole does: no seed is to start there, and there is no
            // curve. (On an open patch it was reported as a curve from the
            // pole to itself, a few tolerances long in the chart's
            // parameters and nowhere in space; an octant against a torus
            // through its pole, tilted, at a million, then lost the arc
            // that divides the torus's face: "imprint lost".)
            let collapsed = {
                let size = Bbox3::from_points(ctrl.iter().copied()).diagonal();
                points
                    .first()
                    .is_some_and(|&first| points.iter().all(|p| p.distance(first) <= 1e-9 * size))
            };
            if collapsed {
                if let Some(flag) = self.lying.get_mut(side) {
                    *flag = true;
                }
                continue;
            }
            let Ok(curve) = NurbsCurve::new(degree, knots, points, curve_weights) else {
                continue;
            };
            if let Some(flag) = self.lying.get_mut(side) {
                *flag = true;
            }
            curves.push(MeetingCurve {
                range: curve.domain(),
                curve: Curve3::Nurbs(curve),
                closed: around,
                tangent: false,
                exact: false,
                deviation: worst.max(4.0 * self.on),
            });
        }
        Ok(curves)
    }

    /// True when a seed sits on a border that lies on the field: the
    /// branch it would start is the border curve, already reported.
    fn on_lying_border(&self, u: f64, v: f64) -> bool {
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let (hair_u, hair_v) = (1e-9 * (u1 - u0), 1e-9 * (v1 - v0));
        (self.lying[0] && (u - u0).abs() <= hair_u)
            || (self.lying[1] && (u - u1).abs() <= hair_u)
            || (self.lying[2] && (v - v0).abs() <= hair_v)
            || (self.lying[3] && (v - v1).abs() <= hair_v)
    }

    /// True when the stationary point is within `reach` (a length) of a
    /// border of the chart that lies on the field.
    fn near_lying_border(&self, s: &Stationary, reach: f64) -> bool {
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        (self.lying[0] && (s.u - u0).abs() * s.lu <= reach)
            || (self.lying[1] && (s.u - u1).abs() * s.lu <= reach)
            || (self.lying[2] && (s.v - v0).abs() * s.lv <= reach)
            || (self.lying[3] && (s.v - v1).abs() * s.lv <= reach)
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
        let mut stack = vec![span];
        while let Some((lo, hi)) = stack.pop() {
            self.spent()?;
            let step = if along_u { grid.du } else { grid.dv };
            let half = 0.5 * (hi - lo) as f64 * step;
            let mid = 0.5 * (lo + hi) as f64;
            let (u, v) = if along_u {
                (grid.u0 + grid.du * mid, grid.v0 + grid.dv * fixed as f64)
            } else {
                (grid.u0 + grid.du * fixed as f64, grid.v0 + grid.dv * mid)
            };
            let b = if along_u {
                self.chart.bounds_in((u - half, u + half), (v, v))
            } else {
                self.chart.bounds_in((u, u), (v - half, v + half))
            };
            let (speed, bend) = if along_u {
                (b.lu, b.muu)
            } else {
                (b.lv, b.mvv)
            };
            let reach = speed * half;
            let monotonic = match self.eval(u, v) {
                Some(e) if e.dist > reach => continue,
                Some(e) => self.field.bend_at(e.p, reach, e.dist).is_some_and(|k| {
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

    /// Seeds where a border of the field's surface crosses the chart's
    /// surface. The field goes on past a border of a freeform surface (the
    /// distance to its nearest point, on the border), and the cells see
    /// only that: a piece of the intersection between two borders, or
    /// between a border of the field and one of the chart, may lie in
    /// cells that are simple because the zero set they hold goes on past
    /// the border at both ends, where every seed on their edges is past
    /// the surface and is dropped (a lofted side cut a corner off the
    /// half of a helmet next to its vent, between the half's meridian and
    /// the vent's rim, and the piece was missing). Such a piece ends on a
    /// border of the field where that border meets the chart's surface:
    /// those points are found along each border that is an edge (not a
    /// seam or a pole), by bisection on the parameter where the distance
    /// to the chart's surface could be zero (it changes no faster than
    /// the border moves), and each gives a seed.
    fn field_border_seeds(&mut self) -> Result<Vec<(f64, f64)>, GeomError> {
        let Some(chart) = self.field_chart.as_ref() else {
            return Ok(Vec::new());
        };
        let Field::Free(own) = &self.own else {
            return Ok(Vec::new());
        };
        let (own, surface) = (std::sync::Arc::clone(own), chart.surface.clone());
        let own_field = Field::Free(std::sync::Arc::clone(&own));
        let (cu, cv, b) = (chart.u, chart.v, chart.bounds);
        let poles = crate::pole::poles(&surface);
        // (Border, parameter along it, its range, speed along it.)
        let mut borders: Vec<(bool, f64, (f64, f64), f64)> = Vec::new();
        if !chart.periodic_u {
            borders.push((true, cu.0, cv, b.lv));
            borders.push((true, cu.1, cv, b.lv));
        }
        if !chart.periodic_v {
            borders.push((false, cv.0, cu, b.lu));
            borders.push((false, cv.1, cu, b.lu));
        }
        let place = |fixed_u: bool, at: f64, t: f64| if fixed_u { (at, t) } else { (t, at) };
        // The walked surface lies in the box of its control points, and a
        // border of the field in the box of its row of them: where those
        // boxes are apart, the border crosses nothing.
        let walked = hull(&self.chart.surface);
        let mut seeds = Vec::new();
        for (fixed_u, at, range, speed) in borders {
            let at_end = if fixed_u { at >= cu.1 } else { at >= cv.1 };
            if let (Some(walked), Some(border)) = (walked, border_hull(&surface, fixed_u, at_end))
                && apart(walked, border, self.cut)
            {
                continue;
            }
            // A border that is one point is no edge.
            let (pu, pv) = place(fixed_u, at, 0.5 * (range.0 + range.1));
            if poles.iter().any(|pole| pole.holds(&surface, pu, pv)) {
                continue;
            }
            let length = speed * (range.1 - range.0);
            if !(length > 0.0 && length.is_finite()) {
                continue;
            }
            // The finest piece looked at: a few tolerances, and no more
            // than 4096 of them along the border.
            let finest = (16.0 * self.tol.linear).max(length / 4096.0);
            let signed = |t: f64| -> Option<(f64, f64, Vec3)> {
                let (u, v) = place(fixed_u, at, t);
                let p = surface.eval(u, v);
                let sample = own_field.sample(p)?;
                Some((sample.signed, sample.dist, p))
            };
            let mut stack = vec![range];
            while let Some((lo, hi)) = stack.pop() {
                self.spent()?;
                self.evals += 1;
                let half = 0.5 * (hi - lo);
                let Some((_, dist, _)) = signed(lo + half) else {
                    continue;
                };
                // Nothing of the border here comes near the chart.
                if dist > speed * half * 1.001 + self.cut {
                    continue;
                }
                if speed * half > finest {
                    stack.push((lo + half, hi));
                    stack.push((lo, lo + half));
                    continue;
                }
                let (Some((mut fa, _, _)), Some((fb, _, _))) = (signed(lo), signed(hi)) else {
                    continue;
                };
                if (fa >= 0.0) == (fb >= 0.0) {
                    continue;
                }
                let (mut a, mut c) = (lo, hi);
                for _ in 0..60 {
                    let m = 0.5 * (a + c);
                    if !(m > a && m < c) {
                        break;
                    }
                    let Some((fm, _, _)) = signed(m) else { break };
                    if (fm >= 0.0) == (fa >= 0.0) {
                        (a, fa) = (m, fm);
                    } else {
                        c = m;
                    }
                }
                let Some((_, dist, p)) = signed(0.5 * (a + c)) else {
                    continue;
                };
                // Past the chart's own edge the crossing is no point of it.
                if dist > self.cut {
                    continue;
                }
                let (u, v, _) = own.foot(p);
                seeds.push((u, v));
            }
        }
        Ok(seeds)
    }

    /// The four sides of a cell, as `(along u, fixed index, from, to)`,
    /// with the fixed index taken around a closed chart.
    fn sides(
        chart: &Chart,
        grid: &Grid,
        (i, j, size): (i64, i64, i64),
    ) -> [(bool, i64, i64, i64); 4] {
        let wrap_u = |k: i64| {
            if chart.periodic_u {
                k.rem_euclid(grid.nu)
            } else {
                k
            }
        };
        let wrap_v = |k: i64| {
            if chart.periodic_v {
                k.rem_euclid(grid.nv)
            } else {
                k
            }
        };
        [
            (true, wrap_v(j), i, i + size),
            (true, wrap_v(j + size), i, i + size),
            (false, wrap_u(i), j, j + size),
            (false, wrap_u(i + size), j, j + size),
        ]
    }

    /// Divides the chart into cells; returns the seeds on the edges of the
    /// cells that may hold a piece of the intersection, and adds the
    /// points where the surfaces are tangent and touch to `singular`.
    fn seeds(&mut self, singular: &mut Vec<Singular>) -> Result<Vec<(f64, f64)>, GeomError> {
        let grid = self.grid();
        let b = self.chart.bounds;
        let mut stack: Vec<(i64, i64, i64)> = Vec::new();
        for i in (0..grid.nu / grid.base).rev() {
            for j in (0..grid.nv / grid.base).rev() {
                stack.push((i * grid.base, j * grid.base, grid.base));
            }
        }
        let mut leaves: Vec<(i64, i64, i64)> = Vec::new();
        // Cells are looked at only where their image can be inside the
        // box: the chart of a freeform surface, a torus or a sphere is the
        // whole surface wherever the box is, and a chart cut to the box
        // (a plane's, a cylinder's, a cone's) reaches out of it at its
        // corners.
        let mut floor = 0_usize;
        let mut cells = 0_usize;
        let merge = 8.0 * self.tol.linear;
        // The size of a floor cell in space.
        let finest = 0.5 * (b.lu * grid.du + b.lv * grid.dv);
        let mut discs: Vec<Disc> = Vec::new();
        // Stationary points already looked at, in space.
        let mut looked: Vec<Vec3> = Vec::new();
        let (mut waiting, mut patience) = (0_usize, FIRST_LOOK);
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
            if discs.iter().any(|d| self.disc_covers(d, u, v, hu, hv)) {
                continue;
            }
            let b = self.chart.bounds_in((u - hu, u + hu), (v - hv, v + hv));
            let reach = b.lu * hu + b.lv * hv;
            let here = self.eval(u, v);
            // The image of the cell lies within `reach` of its centre's:
            // further than that outside the box, none of it is inside.
            let away = |e: &Eval| !self.within.contains(e.p, reach + self.tol.linear);
            // (Unproven: a branch that came into a kept cell from one left
            // out and went back out across the same side between two nodes
            // would show no change of sign there and be missed. No such
            // case has been found (a search of those sides as borders
            // changed nothing on 612 cases tried and was taken out), but it
            // is not ruled out: geometry.md › *Only the box is walked*.)
            if here.as_ref().is_some_and(away) {
                continue;
            }
            let simple = match here {
                Some(e) if e.dist > reach => continue,
                Some(e) => self.field.bend_at(e.p, reach, e.dist).is_some_and(|k| {
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
                continue;
            }
            // Neither empty nor simple: near a point where the surfaces are
            // tangent, or the bounds are loose here. Every floor cell
            // looks for the point; larger cells do when enough of them
            // have piled up.
            waiting += 1;
            if size == 1 || waiting >= patience {
                waiting = 0;
                let at_apex = singular.iter().any(|s| {
                    s.kind == SingularKind::Apex
                        && self.chart.surface.eval(u, v).distance(s.p) <= 2.0 * merge
                });
                let mut settled = false;
                // A touch the Hessian does not classify (along a curve, or
                // of higher order) that keeps to one side all around.
                let found = if at_apex { None } else { self.stationary(u, v) };
                // A tacnode found again (anywhere along its valley: where
                // it lies along its arms is ill-conditioned) has its disc
                // already.
                let found = found.filter(|s| !self.tac_claims(s.u, s.v));
                // A contact of higher order along one direction with arms
                // (a tacnode or a cusp); failing that, one that keeps to
                // one side (a touch).
                // (The search for it takes no step where the Hessian is
                // flat: it goes on along the valley of the strong
                // direction, and the point it finds there is used for
                // the arms only.)
                let arms = match found {
                    Some(s) if s.flat => {
                        let settled = self.settle_flat(&s).unwrap_or(s);
                        if self.touches(&settled, finest) && !self.tac_claims(settled.u, settled.v)
                        {
                            self.tacnode(&settled, finest)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let grazing = match found {
                    Some(s) if arms.is_none() && s.flat && self.touches(&s, finest) => {
                        self.one_sided(&s, finest)
                    }
                    _ => None,
                };
                if let (Some(found), Some(tac)) = (found, arms) {
                    singular.push(Singular {
                        u: tac.u,
                        v: tac.v,
                        p: tac.p,
                        kind: SingularKind::Tacnode,
                        arm: 1.25 * tac.radius,
                        turn: 0.0,
                    });
                    let disc = Disc {
                        u: tac.u,
                        v: tac.v,
                        lu: found.lu,
                        lv: found.lv,
                        radius: tac.radius,
                    };
                    settled = self.disc_covers(&disc, u, v, hu, hv);
                    discs.push(disc);
                    patience = FIRST_LOOK;
                    self.tacnodes.push(tac);
                } else if let (Some(found), Some(disc)) = (found, grazing) {
                    // (On a border of the chart that lies on the other
                    // surface the border is the meeting, already reported
                    // as a curve: that the surfaces are tangent along it,
                    // a wall and its mirror image which continues it, is
                    // no point of its own.)
                    if !self.near_lying_border(&found, 8.0 * finest)
                        && !singular.iter().any(|s| s.p.distance(found.e.p) <= merge)
                    {
                        singular.push(Singular {
                            u: found.u,
                            v: found.v,
                            p: found.e.p,
                            kind: SingularKind::Touch,
                            arm: 0.0,
                            turn: 0.0,
                        });
                    }
                    settled = self.disc_covers(&disc, u, v, hu, hv);
                    discs.push(disc);
                    patience = FIRST_LOOK;
                } else if let Some(found) = found {
                    if let Some(point) = self.singular_at(&found, finest)?
                        && !singular.iter().any(|s| s.p.distance(point.p) <= merge)
                    {
                        singular.push(point);
                    }
                    if !looked.iter().any(|p| p.distance(found.e.p) <= merge) {
                        looked.push(found.e.p);
                        if let Some(disc) = self.disc(&found, finest) {
                            settled = self.disc_covers(&disc, u, v, hu, hv);
                            discs.push(disc);
                            patience = FIRST_LOOK;
                        }
                    }
                }
                if settled {
                    continue;
                }
                if size > 1 {
                    patience = patience.saturating_mul(2);
                }
            }
            if size == 1 {
                leaves.push((i, j, size));
                floor += 1;
                if floor > MAX_FLOOR_CELLS {
                    return Err(too_many());
                }
            } else {
                let h = size / 2;
                for (di, dj) in [(h, h), (0, h), (h, 0), (0, 0)] {
                    stack.push((i + di, j + dj, h));
                }
            }
        }
        // A touch found on the valley of a tacnode before the tacnode
        // itself was is a place along its arms, not a point of its own.
        singular.retain(|s| s.kind != SingularKind::Touch || !self.tac_claims(s.u, s.v));
        // Every edge of every leaf, once.
        let mut edges: BTreeSet<(bool, i64, i64, i64)> = BTreeSet::new();
        for &leaf in &leaves {
            edges.extend(Self::sides(&self.chart, &grid, leaf));
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
                // A border that lies on the field is a curve already.
                let side = match (along_u, fixed == 0) {
                    (true, true) => 2,
                    (true, false) => 3,
                    (false, true) => 0,
                    (false, false) => 1,
                };
                if self.lying.get(side).copied().unwrap_or(false) {
                    continue;
                }
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
        Ok(seeds)
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

    /// Newton on the gradient of `f`: the point near `(u0, v0)` where the
    /// surfaces are tangent (their normals parallel), wherever the other
    /// surface is.
    fn stationary(&mut self, u0: f64, v0: f64) -> Option<Stationary> {
        let (mut u, mut v) = (u0, v0);
        let start = self.eval(u0, v0)?;
        for _ in 0..30 {
            let e = self.eval(u, v)?;
            let (a, b, c) = self.hessian(u, v)?;
            let (lu, lv) = (e.su.length(), e.sv.length());
            if !(lu > 0.0 && lv > 0.0) {
                return None;
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
                let found = Stationary {
                    u,
                    v,
                    e,
                    lu,
                    lv,
                    g: (gu, gv),
                    h: (ha, hb, hc),
                    flat,
                };
                return Some(found);
            }
            u += step_u / lu;
            v += step_v / lv;
            if !(u.is_finite() && v.is_finite()) || e.p.distance(start.p) > 64.0 * self.hmax {
                return None;
            }
        }
        None
    }

    /// True when a disc covers the cell around `(u, v)`, the cell taken
    /// round a periodic chart to the disc's side.
    fn disc_covers(&self, d: &Disc, u: f64, v: f64, hu: f64, hv: f64) -> bool {
        let c = &self.chart;
        let u = Self::unwrap(u, d.u, c.periodic_u, c.u.1 - c.u.0);
        let v = Self::unwrap(v, d.v, c.periodic_v, c.v.1 - c.v.0);
        d.covers(u, v, hu, hv)
    }

    /// The gradient and Hessian of `f` at `(u, v)`, as a stationary point.
    fn stationary_at(&mut self, u: f64, v: f64) -> Option<Stationary> {
        let e = self.eval(u, v)?;
        let (a, b, c) = self.hessian(u, v)?;
        let (lu, lv) = (e.su.length(), e.sv.length());
        if !(lu > 0.0 && lv > 0.0) {
            return None;
        }
        let (ha, hb, hc) = (a / (lu * lu), b / (lu * lv), c / (lv * lv));
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let flat = !((ha * hc - hb * hb).abs() > 1e-3 * size * size);
        Some(Stationary {
            u,
            v,
            e,
            lu,
            lv,
            g: (e.gu / lu, e.gv / lv),
            h: (ha, hb, hc),
            flat,
        })
    }

    /// True when the surfaces touch at the stationary point: nearer to
    /// each other than the tolerance and than a floor cell reaches
    /// (`finest`), which is where the cells alone would have come down to
    /// the floor. Further apart the zero set of `f` keeps clear of the
    /// point (a saddle one tolerance off the other surface has its two
    /// branches thousands of tolerances apart), and cells find it.
    fn touches(&self, s: &Stationary, finest: f64) -> bool {
        let (ha, hb, hc) = s.h;
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let tangent = s.g.0.hypot(s.g.1) <= 1e-6_f64.max(TOUCH * size * self.tol.linear);
        tangent && s.e.dist <= self.tol.linear.min(finest) && self.inside(s.u, s.v)
    }

    /// The stationary point as a point of the intersection, if the
    /// surfaces touch there.
    fn singular_at(&self, s: &Stationary, finest: f64) -> Result<Option<Singular>, GeomError> {
        let (ha, hb, hc) = s.h;
        if !self.touches(s, finest) {
            return Ok(None);
        }
        if s.flat {
            return Err(GeomError::Unsupported(
                "surfaces that touch along a curve, or with more than second-order contact, without a closed form",
            ));
        }
        // Where the surfaces are a little apart at a crossing (within the
        // tolerance, so it is read as one), `f = 0` is two branches that
        // turn away at `sqrt(2 |f| / λ)` from the point, `λ` the weaker
        // eigenvalue of the Hessian: the arms start four times as far out,
        // where the branches run along the directions of the crossing
        // within a thirtieth and a march towards the point ends on it.
        let mean = 0.5 * (ha + hc);
        let weaker = (mean.abs() - (0.25 * (ha - hc) * (ha - hc) + hb * hb).sqrt()).abs();
        let turn = if weaker > 0.0 {
            (2.0 * s.e.f.abs() / weaker).sqrt()
        } else {
            0.0
        };
        Ok(Some(Singular {
            u: s.u,
            v: s.v,
            p: s.e.p,
            kind: if ha * hc - hb * hb > 0.0 {
                SingularKind::Touch
            } else {
                SingularKind::Cross
            },
            arm: self.arm_radius().max(4.0 * turn),
            turn,
        }))
    }

    /// A disc around a point where the surfaces touch and the Hessian of
    /// `f` is degenerate (they touch along a curve through it, or to more
    /// than second order), in which `f` keeps to one side: on three rings
    /// of samples it is nowhere on the other side by more than a quarter
    /// of the tolerance, and on the outer ring mostly clear of zero. The
    /// surfaces then touch there without crossing, which divides nothing:
    /// the point is a touch, and the disc needs no cells. (Going through
    /// each other by less than half the tolerance is a touch to a boolean
    /// anyway.) `None` where `f` changes sign on every ring down to the
    /// smallest: a crossing, which stays refused.
    ///
    /// The rings are a sample, and the radius shrinks until they agree: a
    /// crossing in a wedge narrower than the gaps between the samples (a
    /// cusp, `x² − y³`) passes at a small radius. Nothing is lost by it,
    /// because a disc only keeps cells from being split: the arms of such
    /// a crossing leave the disc, are found outside it and are marched to
    /// the point (`a_degenerate_contact_that_crosses_in_a_cusp_is_not_a_touch`).
    /// What a disc could hide is a closed curve wholly inside it that
    /// every one of its 48 samples misses.
    fn one_sided(&mut self, s: &Stationary, finest: f64) -> Option<Disc> {
        let c = &self.chart;
        let widest = 0.25 * ((c.u.1 - c.u.0) * s.lu).min((c.v.1 - c.v.0) * s.lv);
        let least = 8.0 * finest;
        let slack = self.cut;
        let mut radius = widest;
        'radius: while radius.is_finite() && radius >= least {
            let (mut above, mut below, mut beyond, mut seen) = (0_u32, 0_u32, 0_u32, 0_u32);
            for (ring, count) in [(1.0, 24_u32), (2.0 / 3.0, 16), (1.0 / 3.0, 8)] {
                for k in 0..count {
                    self.spent().ok()?;
                    let angle =
                        std::f64::consts::TAU * (f64::from(k) + 0.5 * ring) / f64::from(count);
                    let (x, y) = (ring * radius * angle.cos(), ring * radius * angle.sin());
                    let (u, v) = (s.u + x / s.lu, s.v + y / s.lv);
                    if !self.inside(u, v) {
                        continue; // No cell lies there.
                    }
                    let Some(e) = self.eval(u, v) else {
                        radius *= 0.5;
                        continue 'radius;
                    };
                    seen += u32::from(ring == 1.0);
                    // Past the edge of the other surface the distance says
                    // nothing about sides: nothing of that surface is
                    // there, on either side. (All around a point of a
                    // border the two surfaces share, where one continues
                    // the other.)
                    if e.dist > e.f.abs() + self.on {
                        beyond += u32::from(e.dist > slack);
                        continue;
                    }
                    above += u32::from(e.f > slack);
                    below += u32::from(e.f < -slack);
                }
            }
            // One side only, and on the rim mostly away from the other
            // surface (not two surfaces that coincide there).
            let (most, least_side) = (above.max(below) + beyond, above.min(below));
            if least_side == 0 && seen > 0 && 2 * most >= seen {
                return Some(Disc {
                    u: s.u,
                    v: s.v,
                    lu: s.lu,
                    lv: s.lv,
                    radius: 0.9 * radius,
                });
            }
            radius *= 0.5;
        }
        None
    }

    /// The largest disc around a stationary point on whose rim, and
    /// halfway to it, the gradient of `f` is what the Hessian at the point
    /// says, within [`DISC_MODEL`]: `f` then has no other critical point
    /// in it, and its zero set there is the point, the two branches
    /// through it, or nothing. `None` where the zero set is not one of
    /// those (a critical point off the other surface around which `f`
    /// changes sign), or where no disc of at least a few floor cells
    /// (`finest` is the size of one) passes.
    fn disc(&mut self, s: &Stationary, finest: f64) -> Option<Disc> {
        let (ha, hb, hc) = s.h;
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let tangent = s.g.0.hypot(s.g.1) <= 1e-6_f64.max(TOUCH * size * self.tol.linear);
        if s.flat || !tangent || !self.inside(s.u, s.v) {
            return None;
        }
        // Off the other surface: only a minimum above it or a maximum
        // below it leaves the disc without a zero. (Past the edge of a
        // bounded surface the distance is to that edge, not a smooth
        // function of the place.)
        let touching = self.touches(s, finest);
        let on = self.on;
        let smooth = move |e: &Eval| e.dist <= e.f.abs() + on;
        if !touching && !(ha * hc - hb * hb > 0.0 && s.e.f * ha > 0.0 && smooth(&s.e)) {
            return None;
        }
        let c = &self.chart;
        let widest = 0.25 * ((c.u.1 - c.u.0) * s.lu).min((c.v.1 - c.v.0) * s.lv);
        let least = 8.0 * finest;
        let mut radius = widest;
        'radius: while radius.is_finite() && radius >= least {
            for (ring, count) in [(1.0, 16_u32), (0.5, 8)] {
                for k in 0..count {
                    self.spent().ok()?;
                    // The inner ring sits between the outer one's places.
                    let angle =
                        std::f64::consts::TAU * (f64::from(k) + 0.5 * ring) / f64::from(count);
                    let (x, y) = (ring * radius * angle.cos(), ring * radius * angle.sin());
                    let (u, v) = (s.u + x / s.lu, s.v + y / s.lv);
                    if !self.inside(u, v) {
                        continue; // No cell lies there.
                    }
                    let model = (ha * x + hb * y, hb * x + hc * y);
                    let holds = self.eval(u, v).is_some_and(|e| {
                        let off = (e.gu / s.lu - model.0).hypot(e.gv / s.lv - model.1);
                        off <= DISC_MODEL * model.0.hypot(model.1)
                            && (touching || (e.f * s.e.f > 0.0 && smooth(&e)))
                    });
                    if !holds {
                        radius *= 0.5;
                        continue 'radius;
                    }
                }
            }
            return Some(Disc {
                u: s.u,
                v: s.v,
                lu: s.lu,
                lv: s.lv,
                radius: 0.9 * radius,
            });
        }
        None
    }

    /// How far from a crossing point its arms are seeded, at the least.
    fn arm_radius(&self) -> f64 {
        (64.0 * self.tol.linear).max(1e-2 * self.hmax)
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
        let radius = s.arm;
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
        let (periodic_u, periodic_v) = (c.periodic_u, c.periodic_v);
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
                    // The other parameter stays in its range, unless it
                    // goes round.
                    let ok = if on_u {
                        periodic_v || (y >= range_v.0 - slack_v && y <= range_v.1 + slack_v)
                    } else {
                        periodic_u || (y >= range_u.0 - slack_u && y <= range_u.1 + slack_u)
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
                // A crossing point is looked for two steps ahead, and from
                // the radius at which its arms are seeded: where the
                // surfaces are a little apart at it the branch turns away
                // just before the point, and a step that lands there would
                // pass it by and go on along the next arm; where they are
                // not, a step that lands on the point itself would.
                let reach = match s.kind {
                    SingularKind::Cross => (2.0 * h).max(s.arm),
                    SingularKind::Tacnode => s.arm,
                    _ => h,
                };
                if !(dist > self.on && dist <= reach && along > 0.0) {
                    continue;
                }
                let (su, sv) = self.near((s.u, s.v), &cur);
                // Inside a tacnode's radius its arms are made, not marched
                // (`tacnode.rs`): every branch heading in ends on it.
                if s.kind == SingularKind::Tacnode {
                    // Only a branch on its arms (`tac_admits`); one that
                    // passes near it goes on.
                    if !self.tac_admits(s.p, cur.u, cur.v) {
                        continue;
                    }
                    // With the direction along: the arms' tangent there.
                    snapped = Some(self.tac_arrival(s.p, (su, sv), d).ok_or(GeomError::NoConvergence(
                        "an arm that ends on a point of higher-order contact has no direction there",
                    ))?);
                    break;
                }
                if (d - cur.t * along).length() > 0.1 * dist {
                    continue;
                }
                // The curve really goes there: its middle is near the chord.
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
            // Near a tacnode the other arm is close: short steps.
            h = self.tac_step(cur.p, h).max(hmin);
            let (pu, pv) = (cur.u + h * cur.du, cur.v + h * cur.dv);
            let Some((e, u, v)) = self.correct(pu, pv) else {
                // Past the border of the chart there may be nothing to
                // correct onto: end on the border if the curve reaches it.
                if !self.inside(pu, pv) {
                    if let Some(end) = self.exit(&cur, pu, pv, sign) {
                        pts.push(end);
                        return Ok((pts, End::Open));
                    }
                    if self.on_border(&cur) {
                        return Ok((pts, End::Open));
                    }
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(e) => return Err(e),
                };
                continue;
            };
            let step = e.p - cur.p;
            let len = step.length();
            let new = Self::point(&e, u, v, sign);
            let turn = new.map_or(-1.0, |n| n.t.dot(cur.t));
            // A step goes about as far as it was asked (`h` along the
            // tangent, to first order): one that ends less than half or
            // more than twice as far went somewhere the prediction did
            // not, such as once around a closed curve smaller than the
            // step, back to just past where it began (the tangent there
            // is the same, so the turn does not show it).
            let Some(new) = new.filter(|_| {
                step.dot(cur.t) > 0.0 && len >= 0.5 * h && len <= 2.0 * h && turn >= cos_turn
            }) else {
                // Off the edge of the other surface the field has a crease
                // and no tangent: that is an end, not a stall.
                if e.dist > self.cut {
                    pts.extend(self.edge(&cur, h, sign));
                    return Ok((pts, End::Open));
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(err) => {
                        // Where the speed of the parametrisation jumps (a
                        // knot of a spline whose spans are drawn at other
                        // paces: rational weights far apart) the step
                        // predicted from the derivatives on one side goes
                        // as far off however short it is. The parameter
                        // step is then scaled to the length it went.
                        let ahead = step.dot(cur.t) > 0.0 && turn >= cos_turn;
                        let Some(new) = ahead
                            .then(|| self.rescaled(&cur, h, len, sign, cos_turn))
                            .flatten()
                        else {
                            return Err(err);
                        };
                        pts.push(new);
                        cur = new;
                        continue;
                    }
                };
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
                if self.on_border(&cur) {
                    return Ok((pts, End::Open)); // It started on the border.
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(e) => return Err(e),
                };
                continue;
            }
            pts.push(new);
            cur = new;
            if turn >= cos_half {
                h = (1.5 * h).min(self.hmax);
            }
        }
    }

    /// A step of about `h` from `cur` where the step predicted along the
    /// parameters' tangent went `len` instead, however short it was made:
    /// the parameter step scaled by the length it went (a secant on its
    /// length), and once one went too short and one too far, halved
    /// between them (the length grows with the scale, at another rate on
    /// either side of a jump). `None` unless one lands as a step must
    /// (about `h` ahead, turning little, on the field and in the chart).
    fn rescaled(&mut self, cur: &Pt, h: f64, len: f64, sign: f64, cos_turn: f64) -> Option<Pt> {
        let (mut scale, mut went) = (h, len);
        let (mut short, mut far): (Option<f64>, Option<f64>) = (None, None);
        for _ in 0..RESCALES {
            if !(went > 0.0 && went.is_finite()) {
                return None;
            }
            if went < 0.5 * h {
                short = Some(scale);
            } else {
                far = Some(scale);
            }
            scale = match (short, far) {
                (Some(a), Some(b)) => 0.5 * (a + b),
                _ => scale * h / went,
            };
            if !(scale > 0.0 && scale.is_finite()) {
                return None;
            }
            let (e, u, v) = self.correct(cur.u + scale * cur.du, cur.v + scale * cur.dv)?;
            let step = e.p - cur.p;
            went = step.length();
            let new = Self::point(&e, u, v, sign)?;
            if !(step.dot(cur.t) > 0.0 && new.t.dot(cur.t) >= cos_turn) {
                return None;
            }
            if went >= 0.5 * h && went <= 2.0 * h {
                return (e.dist <= self.cut && self.inside(u, v)).then_some(new);
            }
        }
        None
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
        // An end on a border moved in short of a cusp is drawn on to it.
        if !closed {
            if let Some(last) = pts.last().copied()
                && let Some(cusp) = self.cusp_of(&last)
            {
                let more = self.on_into_cusp(&last, &cusp)?;
                pts.extend(more);
            }
            if let Some(first) = pts.first().copied()
                && let Some(cusp) = self.cusp_of(&first)
            {
                let more = self.on_into_cusp(&first.reversed(), &cusp)?;
                let mut before: Vec<Pt> = more.into_iter().rev().map(Pt::reversed).collect();
                before.append(&mut pts);
                pts = before;
            }
        }
        self.refine(&mut pts)?;
        Ok(Some(Branch { pts, closed }))
    }

    /// The border moved in short of a cusp that `p` is on, if any.
    fn cusp_of(&self, p: &Pt) -> Option<Cusp> {
        let c = &self.chart;
        c.cusps.iter().copied().find(|cusp| {
            let (here, (lo, hi)) = if cusp.pole.across_v() {
                (p.v, c.v)
            } else {
                (p.u, c.u)
            };
            (here - cusp.at).abs() <= 1e-9 * (hi - lo)
        })
    }

    /// The rest of a branch that ends at `end` on a border moved in short
    /// of a cusp (`field.rs`, `short_of_poles`), on to the point: at
    /// places across ever nearer to the pole (halving the distance), the
    /// zero of `f` along the border's parameter next to the last one, by
    /// bracketing and regula falsi on its sign alone (there `f` and its
    /// derivative along vanish to the order of the cusp, so no step size
    /// or test in units of length applies, but the sign does), until the
    /// place is within `on` of the pole by the speed across or every
    /// iso-curve along is shorter than `on` (nearer than that the sign of
    /// `f` is rounding, and what is left is within `on` of one meridian);
    /// then the pole, with the direction the meridians leave it along. `end`
    /// travels into the pole; the points come in that order.
    ///
    /// Refused where the zero is lost on the way (the intersection turns
    /// back before the point), or the other surface ends there.
    fn on_into_cusp(&mut self, end: &Pt, cusp: &Cusp) -> Result<Vec<Pt>, GeomError> {
        let lost = GeomError::Unsupported(
            "an intersection curve that comes close to a cusp of a surface and turns back before it",
        );
        let across_v = cusp.pole.across_v();
        let ((u0, u1), periodic) = if across_v {
            (self.chart.u, self.chart.periodic_u)
        } else {
            (self.chart.v, self.chart.periodic_v)
        };
        let place = |along: f64, across: f64| {
            if across_v {
                (along, across)
            } else {
                (across, along)
            }
        };
        let speed = if across_v {
            self.chart.bounds.lv
        } else {
            self.chart.bounds.lu
        };
        let length = u1 - u0;
        // Not wrapped round a periodic chart: the points continue the
        // branch's parameters (the spans between them are halved in
        // them), and every evaluation takes them round into the domain
        // (`Chart::at`).
        let mut along = if across_v { end.u } else { end.v };
        let mut previous = cusp.at;
        let mut pts: Vec<Pt> = Vec::new();
        let mut last = *end;
        let gap = cusp.at - cusp.from;
        let n = cusp.narrowing;
        for k in 1..=80 {
            self.spent()?;
            let across = cusp.from + gap * 0.5_f64.powi(k);
            // Nearer than this every iso-curve along is shorter than `on`
            // (the rest is one line to that, and `f` is rounding there),
            // or the pole is within `on`.
            let share = (across - cusp.from).abs() / n.span;
            if n.width * share.powi(n.order as i32) <= self.on
                || speed * (across - cusp.from).abs() <= self.on
            {
                break;
            }
            let f = |m: &mut Self, y: f64| -> Option<Eval> {
                let (u, v) = place(y, across);
                m.eval(u, v)
            };
            // The zero continues the last one: along the border it moves
            // no further than the derivatives along it turn, at most
            // `twist / speed` per unit across (`pole::Narrowing`).
            let limit = (n.twist * (across - previous).abs() / n.speed.max(f64::MIN_POSITIVE))
                .min(0.25 * length);
            previous = across;
            let here = f(self, along).ok_or(lost.clone())?;
            // The nearest change of sign along, either way.
            // On the other surface already, as `correct` takes a point
            // (near the point `f` changes along by less than rounding).
            let mut found = None;
            if here.f.abs() <= self.on {
                found = Some((along, along));
            }
            let mut reach = (1e-9 * length).min(limit);
            while found.is_none() && reach <= limit {
                for side in [-1.0, 1.0] {
                    let y = along + side * reach;
                    if !periodic && !(y >= u0 && y <= u1) {
                        continue;
                    }
                    let other = f(self, y).ok_or(lost.clone())?;
                    if (other.f >= 0.0) != (here.f >= 0.0) {
                        found = Some(if side < 0.0 { (y, along) } else { (along, y) });
                        break;
                    }
                }
                if reach >= limit {
                    break;
                }
                reach = (reach * 4.0).min(limit);
            }
            let (mut lo, mut hi) = found.ok_or(lost.clone())?;
            let (mut flo, mut fhi) = (
                f(self, lo).ok_or(lost.clone())?.f,
                f(self, hi).ok_or(lost.clone())?.f,
            );
            let mut side = 0;
            for step in 0..100 {
                if lo == hi {
                    break;
                }
                if flo == 0.0 {
                    hi = lo;
                    break;
                }
                if fhi == 0.0 {
                    lo = hi;
                    break;
                }
                let y = if step % 4 == 3 {
                    0.5 * (lo + hi)
                } else {
                    (lo * fhi - hi * flo) / (fhi - flo)
                };
                let y = if y > lo && y < hi { y } else { 0.5 * (lo + hi) };
                if !(y > lo && y < hi) {
                    break;
                }
                let fy = f(self, y).ok_or(lost.clone())?.f;
                if (fy >= 0.0) == (flo >= 0.0) {
                    (lo, flo) = (y, fy);
                    if side == -1 {
                        fhi *= 0.5;
                    }
                    side = -1;
                } else {
                    (hi, fhi) = (y, fy);
                    if side == 1 {
                        flo *= 0.5;
                    }
                    side = 1;
                }
            }
            along = 0.5 * (lo + hi);
            let (u, v) = place(along, across);
            let e = self.eval(u, v).ok_or(lost.clone())?;
            if e.dist > self.cut {
                return Err(lost);
            }
            let toward = cusp.pole.point - e.p;
            let point = Self::point(&e, u, v, 1.0)
                .map(|pt| {
                    if pt.t.dot(toward) < 0.0 {
                        pt.reversed()
                    } else {
                        pt
                    }
                })
                .or_else(|| {
                    (e.p - last.p).normalized().map(|t| Pt {
                        u,
                        v,
                        p: e.p,
                        t,
                        du: 0.0,
                        dv: 0.0,
                    })
                })
                .ok_or(lost.clone())?;
            pts.push(point);
            last = point;
        }
        // The point itself: every meridian leaves it along one line, the
        // derivative across there (the same on all of them at a cusp).
        let (u, v) = place(along, cusp.from);
        let (_, su, sv) = self.chart.at(u, v);
        let out = if across_v { sv } else { su };
        let chord = (cusp.pole.point - last.p).normalized();
        let t = match (out.normalized(), chord) {
            (Some(d), Some(c)) => {
                if d.dot(c) < 0.0 {
                    -d
                } else {
                    d
                }
            }
            (Some(d), None) => {
                if d.dot(last.t) < 0.0 {
                    -d
                } else {
                    d
                }
            }
            (None, Some(c)) => c,
            (None, None) => last.t,
        };
        let rate = out.length();
        let step = if rate > 0.0 {
            (cusp.from - cusp.at).signum() / rate
        } else {
            0.0
        };
        let (du, dv) = if across_v { (0.0, step) } else { (step, 0.0) };
        pts.push(Pt {
            u,
            v,
            p: cusp.pole.point,
            t,
            du,
            dv,
        });
        Ok(pts)
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
                    let (u, v) = self
                        .chart
                        .wrapped(a.u + (b.u - a.u) * s, a.v + (b.v - a.v) * s);
                    let (_, _, foot) = projector.foot_from(q, u, v);
                    foot.distance(q)
                }
                field => field.sample(q).map_or(f64::INFINITY, |x| x.dist),
            };
            self.evals += 2;
            worst = worst.max(other).max(own);
        }
        // Where the surfaces are nearly tangent a curve can be close to
        // both and still far from where they meet: hold the middle of the
        // span against a point of the intersection itself. (Not on the
        // way into a crossing point the surfaces are a little apart at:
        // there the curve is drawn to the point, and `f = 0` is not.)
        if !self.modelled(a, b)
            && let Some(m) = self.between(a, b)
        {
            let off = Self::bezier(&c, 0.5) - m.p;
            worst = worst.max((off - m.t * off.dot(m.t)).length());
        }
        worst
    }

    /// True when the span from `a` to `b` lies in the zone of a crossing
    /// point the surfaces are a little apart at.
    fn modelled(&self, a: &Pt, b: &Pt) -> bool {
        self.zones
            .iter()
            .any(|&(p, radius)| a.p.distance(p) <= radius && b.p.distance(p) <= radius)
    }

    /// The point of the chart halfway between `a` and `b` in its
    /// parameters, travelling from `a` to `b`: on the way into a crossing
    /// point along the direction in which the Hessian vanishes, `f` stays
    /// what it is at the point to third order.
    fn straight(&mut self, a: &Pt, b: &Pt) -> Option<Pt> {
        let (u, v) = (0.5 * (a.u + b.u), 0.5 * (a.v + b.v));
        let e = self.eval(u, v)?;
        let t = (b.p - a.p).normalized()?;
        let (g11, g12, g22) = (e.su.dot(e.su), e.su.dot(e.sv), e.sv.dot(e.sv));
        let det = g11 * g22 - g12 * g12;
        if !(det > 0.0) {
            return None;
        }
        let (tu, tv) = (t.dot(e.su), t.dot(e.sv));
        let (du, dv) = ((g22 * tu - g12 * tv) / det, (g11 * tv - g12 * tu) / det);
        (du != 0.0 || dv != 0.0).then_some(Pt {
            u,
            v,
            p: e.p,
            t,
            du,
            dv,
        })
    }

    /// A point of the branch between `a` and `b`.
    fn between(&mut self, a: &Pt, b: &Pt) -> Option<Pt> {
        if let Some(made) = self.tac_between(a, b) {
            return made;
        }
        let (mu, mv) = (0.5 * (a.u + b.u), 0.5 * (a.v + b.v));
        self.between_from(a, b, mu, mv).or_else(|| {
            // Where the speed of the parametrisation jumps between `a`
            // and `b` (a knot of a spline whose spans are drawn at other
            // paces: rational weights far apart) the middle of their
            // parameters may be next to one of them, or Newton from it
            // may not settle: start from the foot of the middle of the
            // chord instead.
            let Field::Free(projector) = &self.own else {
                return None;
            };
            let (u, v, _) = projector.foot(a.p.lerp(b.p, 0.5));
            let (pu, pv) = (
                self.chart.u.1 - self.chart.u.0,
                self.chart.v.1 - self.chart.v.0,
            );
            let u = Self::unwrap(u, mu, self.chart.periodic_u, pu);
            let v = Self::unwrap(v, mv, self.chart.periodic_v, pv);
            self.between_from(a, b, u, v)
        })
    }

    /// The point of the branch between `a` and `b` that Newton finds from
    /// `(u, v)`, if it is between them.
    fn between_from(&mut self, a: &Pt, b: &Pt, u: f64, v: f64) -> Option<Pt> {
        let (e, u, v) = self.correct(u, v)?;
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
        while let (Some(mut a), Some(mut b)) = (pts.get(i).copied(), pts.get(i + 1).copied()) {
            self.spent()?;
            // An end on a crossing point or an apex has no tangent of its
            // own: it gets the one a circle through its neighbour would
            // have there (the neighbour's, mirrored in the chord).
            let mirrored = |from: &Pt, at: &Pt| {
                (at.p - from.p)
                    .normalized()
                    .map(|chord| chord * (2.0 * chord.dot(from.t)) - from.t)
            };
            if b.du == 0.0 && b.dv == 0.0 && !(a.du == 0.0 && a.dv == 0.0) {
                b.t = mirrored(&a, &b).unwrap_or(b.t);
                if let Some(slot) = pts.get_mut(i + 1) {
                    *slot = b;
                }
            } else if a.du == 0.0 && a.dv == 0.0 && !(b.du == 0.0 && b.dv == 0.0) {
                a.t = mirrored(&b, &a).unwrap_or(a.t);
                if let Some(slot) = pts.get_mut(i) {
                    *slot = a;
                }
            }
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
            let middle = if self.modelled(&a, &b) {
                self.straight(&a, &b)
            } else {
                self.between(&a, &b)
            };
            pts.insert(i + 1, middle.ok_or(give_up)?);
        }
        Ok(())
    }

    /// True when `p` lies on the branch.
    fn on_branch(&self, p: Vec3, pts: &[Pt], slack: f64) -> bool {
        for (k, end) in [pts.first(), pts.last()].into_iter().flatten().enumerate() {
            // A seed just past an end is the same branch.
            let ahead = (p - end.p).dot(end.t) * if k == 0 { -1.0 } else { 1.0 };
            if p.distance(end.p) <= 2.0 * self.cut && ahead >= 0.0 {
                return true;
            }
        }
        Self::on_spans(p, pts, slack)
    }

    /// True when `p` lies on one of the spans between the points.
    fn on_spans(p: Vec3, pts: &[Pt], slack: f64) -> bool {
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

    /// The point a fraction `share` of the way along the spans of `pts`,
    /// by the lengths of their chords.
    fn along(pts: &[Pt], share: f64) -> Option<Vec3> {
        let total: f64 = pts
            .windows(2)
            .map(|w| match w {
                [a, b] => a.p.distance(b.p),
                _ => 0.0,
            })
            .sum();
        let mut left = share * total;
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = a.p.distance(b.p);
            if left <= chord && chord > 0.0 {
                return Some(Self::bezier(&Self::span(a, b), left / chord));
            }
            left -= chord;
        }
        pts.last().map(|p| p.p)
    }

    /// The span of `pts` that `p` lies on within `slack`, and where along
    /// it (`(k, s)`: on the cubic from `pts[k]` to `pts[k + 1]` at `s`). The
    /// cubic is searched, not its chord: two branches near each other are
    /// apart by less than the chord strays from them.
    fn place_on(p: Vec3, pts: &[Pt], slack: f64) -> Option<(usize, f64)> {
        let mut best: Option<(f64, usize, f64)> = None;
        for (k, (a, b)) in pts.iter().zip(pts.iter().skip(1)).enumerate() {
            let len = a.p.distance(b.p);
            if !(len > 0.0) {
                continue;
            }
            // The span lies within a third of its chord of the chord.
            let chord = b.p - a.p;
            let t = ((p - a.p).dot(chord) / (len * len)).clamp(0.0, 1.0);
            if (a.p + chord * t).distance(p) > 0.5 * len + slack {
                continue;
            }
            let c = Self::span(a, b);
            let at = |s: f64| Self::bezier(&c, s).distance(p);
            // The nearest of 32 samples, then golden sections about it.
            let mut s0 = 0.0;
            for i in 0..=32 {
                let s = f64::from(i) / 32.0;
                if at(s) < at(s0) {
                    s0 = s;
                }
            }
            let (mut lo, mut hi) = ((s0 - 1.0 / 32.0).max(0.0), (s0 + 1.0 / 32.0).min(1.0));
            for _ in 0..60 {
                let (m1, m2) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
                if at(m1) <= at(m2) {
                    hi = m2;
                } else {
                    lo = m1;
                }
            }
            let s = 0.5 * (lo + hi);
            let d = at(s);
            if d <= slack && best.is_none_or(|(least, _, _)| d < least) {
                best = Some((d, k, s));
            }
        }
        best.map(|(_, k, s)| (k, s))
    }

    /// One branch of what two traces found, when they are the same branch:
    /// the second traced from a seed that the first passed by (its end was
    /// decided where the curve runs within the tolerance of the chart's
    /// border, of a pole or of the field's edge, so where along that
    /// stretch it ends is rounding, and a seed further along it is not
    /// "just past" the end). Two traces of one branch run along each other:
    /// they are the same when the quarter points of either lie on the
    /// other within the tolerance. Then the result is the first with what
    /// the second reaches beyond either of its ends (an end of the first
    /// that lies on the second is one the second runs past), refined at
    /// the joints as any span. `None` when they are not one.
    fn united(&mut self, old: &Branch, new: &Branch) -> Result<Option<Branch>, GeomError> {
        if old.closed || new.closed {
            return Ok(None);
        }
        let slack = self.tol.linear;
        // A quarter point of one on the other, with where it is on both.
        let shared = |a: &[Pt], b: &[Pt]| -> Option<Vec3> {
            let mut found = None;
            for share in [0.25, 0.5, 0.75] {
                let q = Self::along(a, share)?;
                Self::place_on(q, b, slack)?;
                found = Some(q);
            }
            found
        };
        let Some(q) = shared(&new.pts, &old.pts).or_else(|| shared(&old.pts, &new.pts)) else {
            return Ok(None);
        };
        // The second the way the first runs: by their spans through `q`.
        let (Some((ko, _)), Some((kn, _))) = (
            Self::place_on(q, &old.pts, slack),
            Self::place_on(q, &new.pts, slack),
        ) else {
            return Ok(None);
        };
        let direction = |pts: &[Pt], k: usize| match (pts.get(k), pts.get(k + 1)) {
            (Some(a), Some(b)) => b.p - a.p,
            _ => Vec3::ZERO,
        };
        let mut pts = new.pts.clone();
        if direction(&old.pts, ko).dot(direction(&new.pts, kn)) < 0.0 {
            pts = pts.into_iter().rev().map(Pt::reversed).collect();
        }
        let (Some(first), Some(last)) = (old.pts.first().copied(), old.pts.last().copied()) else {
            return Ok(None);
        };
        // What the second reaches beyond the first's ends: its points
        // before the span the first's start lies on, and after the one its
        // end lies on.
        let head: Vec<Pt> = match Self::place_on(first.p, &pts, slack) {
            Some((k, _)) => pts.get(..=k).unwrap_or(&[]).to_vec(),
            None => Vec::new(),
        };
        let tail: Vec<Pt> = match Self::place_on(last.p, &pts, slack) {
            Some((k, _)) => pts.get(k + 1..).unwrap_or(&[]).to_vec(),
            None => Vec::new(),
        };
        // (Points of the second at the first's ends are the first's.)
        let beyond = |list: Vec<Pt>| -> Vec<Pt> {
            list.into_iter()
                .filter(|p| Self::place_on(p.p, &old.pts, slack).is_none())
                .collect()
        };
        let (head, tail) = (beyond(head), beyond(tail));
        if head.is_empty() && tail.is_empty() {
            // Nothing of the second beyond the first: no second trace of
            // it. Its seed was not on the first (`on_branch`, a looser
            // test than this one, would have said so), so it is another
            // branch that runs within the tolerance of the first all its
            // short length: an arm that leaves a point of contact along
            // it, cut short by the box. Kept.
            return Ok(None);
        }
        // Parameters continued from the first's ends (a chart closed on
        // itself is unwrapped).
        let shifted = |list: Vec<Pt>, to: &Pt, at: Option<&Pt>, this: &Self| -> Vec<Pt> {
            let Some(at) = at else { return list };
            let (u, v) = this.near((at.u, at.v), to);
            let (du, dv) = (u - at.u, v - at.v);
            list.into_iter()
                .map(|p| Pt {
                    u: p.u + du,
                    v: p.v + dv,
                    ..p
                })
                .collect()
        };
        // An end of the first that the second runs past is no end: where
        // the march stopped there (on the chart's border, at the field's
        // edge) the curve was within the tolerance of it and went on, so
        // that point is the border's, not the curve's, and is left out.
        let mut middle = old.pts.as_slice();
        if !head.is_empty() && middle.len() > 2 {
            middle = middle.get(1..).unwrap_or(middle);
        }
        if !tail.is_empty() && middle.len() > 2 {
            middle = middle.get(..middle.len() - 1).unwrap_or(middle);
        }
        let head = shifted(head.clone(), &first, head.last(), self);
        let tail = shifted(tail.clone(), &last, tail.first(), self);
        let mut joined = head;
        joined.extend(middle.iter().copied());
        joined.extend(tail);
        self.refine(&mut joined)?;
        Ok(Some(Branch {
            pts: joined,
            closed: false,
        }))
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
        let lying = self.border_curves()?;
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
                arm: 0.0,
                turn: 0.0,
            });
        }
        let merge = 8.0 * self.tol.linear;
        let mut seeds = self.seeds(&mut singular)?;
        seeds.extend(self.field_border_seeds()?);
        seeds.retain(|&(u, v)| !self.on_lying_border(u, v));
        for s in singular.clone() {
            if s.kind == SingularKind::Cross {
                self.arms(&s, &mut seeds);
                // (A quarter over: an arm's seed is brought onto `f = 0`
                // from its place on the circle.)
                if s.turn > 0.5 * self.tol.linear {
                    self.zones.push((s.p, 1.25 * s.arm));
                }
            }
        }
        for tac in &self.tacnodes {
            seeds.extend(tac.seeds.iter().copied());
        }
        let mut branches: Vec<Branch> = Vec::new();
        for seed in seeds {
            self.spent()?;
            // On the curve first: a seed is only as good as its bracket.
            let Some((e, u, v)) = self.correct(seed.0, seed.1) else {
                continue;
            };
            // Where the surfaces are nearly tangent a point with `f = 0`
            // within `on` is only that well placed across the curve.
            let sine =
                e.su.cross(e.sv)
                    .normalized()
                    .map_or(0.0, |n| n.cross(e.n).length());
            let slack = self.tol.linear + 16.0 * self.on / sine.max(1e-12);
            // Close to a crossing point every arm has its own seed.
            let covered = |s: &Singular| {
                let radius = match s.kind {
                    SingularKind::Cross | SingularKind::Tacnode => 0.5 * s.arm,
                    _ => 2.0 * merge,
                };
                e.p.distance(s.p) <= radius.max(2.0 * merge)
            };
            if branches.iter().any(|b| self.on_branch(e.p, &b.pts, slack))
                || singular.iter().any(covered)
                || self.tac_inside(u, v)
            {
                continue;
            }
            if let Some(branch) = self.trace((u, v), &singular)? {
                let mut joined = false;
                for old in &mut branches {
                    if let Some(one) = self.united(old, &branch)? {
                        *old = one;
                        joined = true;
                        break;
                    }
                }
                if joined {
                    continue;
                }
                if branches.len() >= MAX_BRANCHES {
                    return Err(GeomError::NoConvergence(
                        "the intersection has more branches than the cap allows",
                    ));
                }
                branches.push(branch);
            }
        }
        let mut curves = lying;
        for branch in &branches {
            let whole = branch.pts.iter().all(|p| within.contains(p.p, 0.0));
            for (mut pts, closed) in self.clip(branch, within) {
                // Where the box cut the branch, the span to the new end is
                // a new cubic (the end and its tangent are new), held to
                // nothing yet: it is refined as the branch was.
                if !whole {
                    self.refine(&mut pts)?;
                }
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
        // Against a freeform surface, which ends: a point whose nearest
        // point of it is on its border is past it (a patch lying on
        // another that ends sooner), and says nothing. Enough of the
        // others must be on it.
        let mut counted = 0_usize;
        for i in 0..9 {
            for j in 0..9 {
                let u = chart.u.0 + (chart.u.1 - chart.u.0) * (f64::from(i) + 0.5) / 9.0;
                let v = chart.v.0 + (chart.v.1 - chart.v.0) * (f64::from(j) + 0.5) / 9.0;
                let p = chart.surface.eval(u, v);
                if let Field::Free(projector) = field
                    && projector.on_border(p)
                {
                    continue;
                }
                let sample = field.sample(p)?;
                if sample.dist > tol.linear {
                    return None;
                }
                counted += 1;
                if same.is_none()
                    && let Some(n) = chart.surface.normal(u, v)
                {
                    same = Some(n.dot(sample.normal) > 0.0);
                }
            }
        }
        // (Every sample of an analytic field counts.)
        (counted >= LIES_ON_COUNTED).then_some(same).flatten()
    }
}

/// Of the 81 samples by which a surface is found to lie on another, how
/// many must be over it (not past a border of a freeform one).
const LIES_ON_COUNTED: usize = 16;

/// The corners of the box of the control points of a freeform surface,
/// which holds the surface (its weights are positive); `None` for the
/// other kinds.
fn hull(surface: &Surface) -> Option<(Vec3, Vec3)> {
    let points: Vec<Vec3> = match surface {
        Surface::Bezier(b) => b.control_points().to_vec(),
        Surface::Nurbs(s) => s.control_points().to_vec(),
        _ => return None,
    };
    Bbox3::from_points(points).corners()
}

/// The corners of the box of the row of control points along a border of
/// a freeform surface (`fixed_u`: the border where u is at its start or
/// `at_end`), which holds the border where the knots are clamped there;
/// `None` otherwise.
fn border_hull(surface: &Surface, fixed_u: bool, at_end: bool) -> Option<(Vec3, Vec3)> {
    let spline = match surface {
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok()?,
        Surface::Nurbs(s) => s.clone(),
        _ => return None,
    };
    let (degree_u, degree_v) = spline.degrees();
    let (count_u, count_v) = spline.counts();
    let clamped = |knots: &[f64], degree: usize, count: usize| {
        if at_end {
            knots.get(count) == knots.get(count + degree)
        } else {
            knots.first() == knots.get(degree)
        }
    };
    let ctrl = spline.control_points();
    let row: Vec<Vec3> = if fixed_u {
        if !clamped(spline.knots_u(), degree_u, count_u) {
            return None;
        }
        let i = if at_end { count_u.checked_sub(1)? } else { 0 };
        (0..count_v)
            .filter_map(|j| ctrl.get(i * count_v + j).copied())
            .collect()
    } else {
        if !clamped(spline.knots_v(), degree_v, count_v) {
            return None;
        }
        let j = if at_end { count_v.checked_sub(1)? } else { 0 };
        (0..count_u)
            .filter_map(|i| ctrl.get(i * count_v + j).copied())
            .collect()
    };
    Bbox3::from_points(row).corners()
}

/// True when two boxes are more than `margin` apart along an axis.
fn apart((alo, ahi): (Vec3, Vec3), (blo, bhi): (Vec3, Vec3), margin: f64) -> bool {
    alo.x > bhi.x + margin
        || blo.x > ahi.x + margin
        || alo.y > bhi.y + margin
        || blo.y > ahi.y + margin
        || alo.z > bhi.z + margin
        || blo.z > ahi.z + margin
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::{Cylinder, Plane};
    use craft3d_math::Frame;

    /// A marcher on the plane z = 0 (its chart is x and y) at scale `k`.
    fn marcher(k: f64) -> Marcher {
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let axis = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        let rod = Surface::Cylinder(Cylinder::new(axis, k).unwrap());
        let within = Bbox3::from_points([
            Vec3::new(-3.0, -3.0, -3.0) * k,
            Vec3::new(3.0, 3.0, 3.0) * k,
        ]);
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let field = Field::new(&rod).unwrap();
        let chart = Chart::new(&plane, &within, &tol, &field).unwrap().unwrap();
        Marcher::new(chart, field, &within, &tol).unwrap()
    }

    /// The arc of the circle about `centre` (in the plane) of `radius` over
    /// the angles `from..to`, as a branch.
    fn arc(centre: (f64, f64), radius: f64, from: f64, to: f64) -> Branch {
        let pts = (0..=40)
            .map(|i| {
                let a = from + (to - from) * f64::from(i) / 40.0;
                let (s, c) = a.sin_cos();
                let (x, y) = (centre.0 + radius * c, centre.1 + radius * s);
                Pt {
                    u: x,
                    v: y,
                    p: Vec3::new(x, y, 0.0),
                    t: Vec3::new(-s, c, 0.0),
                    du: -s,
                    dv: c,
                }
            })
            .collect();
        Branch { pts, closed: false }
    }

    /// Two branches that touch, or run within a few tolerances of each
    /// other, are not one: circles tangent at a point (as the arms of a
    /// tacnode leave theirs), arcs of one radius about centres three
    /// tolerances apart, and a short arc tangent to a long one that lies
    /// within the tolerance of it all its length (an arm cut short by the
    /// box) without reaching past either end of it. At three scales.
    #[test]
    fn branches_that_only_touch_or_run_close_are_not_one() {
        for k in [1e-3, 1.0, 1e3] {
            let mut m = marcher(k);
            let tol = m.tol.linear;
            // Tangent at (k, 0), a hundredth of the radius apart across.
            let inner = arc((0.0, 0.0), k, -0.5, 0.5);
            let outer = arc((-0.01 * k, 0.0), 1.01 * k, -0.5, 0.5);
            assert!(
                m.united(&inner, &outer).unwrap().is_none(),
                "touching at {k}"
            );
            assert!(
                m.united(&outer, &inner).unwrap().is_none(),
                "touching at {k}"
            );
            // Three tolerances apart all along.
            let beside = arc((3.0 * tol, 0.0), k, -0.5, 0.5);
            assert!(
                m.united(&inner, &beside).unwrap().is_none(),
                "beside at {k}"
            );
            // The tangent circle's arc, short: within 2e-8 of the inner
            // circle all along (a hundredth of the radius over 2 times its
            // angle squared), well within the tolerance.
            let short = arc((-0.01 * k, 0.0), 1.01 * k, -0.002, 0.002);
            assert!(
                m.united(&inner, &short).unwrap().is_none(),
                "short arm at {k}"
            );
        }
    }
}
